//! Versioned, bounded pipe IPC. One child per Store cluster, with no JS in parent.
//! Native callers receive tickets; process spawning, reads and waits use workers.
#[path = "store_sandbox.rs"]
mod sandbox;
use crate::{
    runtime::{SharedCompletion, complete, ticket},
    *,
};
pub use sandbox::{PlatformStoreSandbox, STORE_SEATBELT, StoreSandbox};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
const PROTOCOL: u64 = 1;
const FRAME_LIMIT: usize = 64 * 1024 * 1024;
fn io_error(e: impl std::fmt::Display) -> Error {
    Error::Host(e.to_string())
}
fn write_frame(writer: &mut impl Write, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io_error)?;
    if bytes.len() > FRAME_LIMIT {
        return Err(Error::Quota("Store IPC frame"));
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .map_err(io_error)?;
    writer.write_all(&bytes).map_err(io_error)?;
    writer.flush().map_err(io_error)
}
fn read_frame(reader: &mut impl Read) -> Result<Value> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix).map_err(io_error)?;
    let len = u32::from_le_bytes(prefix) as usize;
    if len == 0 || len > FRAME_LIMIT {
        return Err(Error::Quota("Store IPC frame"));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).map_err(io_error)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(io_error)?;
    if value["v"] != PROTOCOL {
        return Err(Error::Invalid("Store IPC version"));
    }
    Ok(value)
}
fn number(v: &Value, key: &str) -> Result<u64> {
    v[key].as_u64().ok_or(Error::Invalid("Store IPC integer"))
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key].as_str().ok_or(Error::Invalid("Store IPC string"))
}
fn wire_generation(g: &Generation) -> Value {
    json!({"realm":g.realm,"number":g.number})
}
fn generation(v: &Value) -> Result<Generation> {
    Ok(Generation {
        realm: string(v, "realm")?.into(),
        number: number(v, "number")?,
    })
}
fn memory(v: &IsolateMemory) -> Value {
    json!({"used":v.used_heap_bytes,"physical":v.physical_heap_bytes,"external":v.external_bytes,"fetch_reserved":v.fetch_reserved_bytes,"engine_reserved":v.engine_reserved_bytes,"snapshot":v.snapshot_bytes})
}
fn error_wire(e: Error) -> Value {
    match e {
        Error::JavaScript(s) => json!({"kind":"js","message":s}),
        Error::Terminated => json!({"kind":"terminated"}),
        Error::Quota(s) => json!({"kind":"quota","message":s}),
        other => json!({"kind":"host","message":other.to_string()}),
    }
}
fn wire_result(v: &Value) -> Result<Value> {
    if v["error"].is_null() {
        Ok(v["value"].clone())
    } else {
        Err(match v["error"]["kind"].as_str() {
            Some("js") => Error::JavaScript(v["error"]["message"].as_str().unwrap_or("").into()),
            Some("terminated") => Error::Terminated,
            Some("quota") => match v["error"]["message"].as_str() {
                Some("heap") => Error::Quota("heap"),
                Some("external memory") => Error::Quota("external memory"),
                _ => Error::Quota("Store remote quota"),
            },
            _ => Error::Host(v["error"].to_string()),
        })
    }
}
fn response(id: u64, result: Result<Value>) -> Value {
    match result {
        Ok(value) => json!({"v":PROTOCOL,"kind":"reply","id":id,"value":value}),
        Err(error) => json!({"v":PROTOCOL,"kind":"reply","id":id,"error":error_wire(error)}),
    }
}
#[derive(Debug, Clone)]
pub struct StoreStatus {
    pub pid: u32,
    pub closed: bool,
    pub confinement: Value,
    /// Last owner-sampled V8 measurement; separate from process RSS.
    pub memory: Value,
    pub resident_bytes: Option<usize>,
}
fn resident_bytes(pid: u32) -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        status.lines().find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|v| v.split_whitespace().next()?.parse::<usize>().ok())
                .map(|kb| kb * 1024)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}
struct StoreInner {
    tx: SyncSender<ParentEvent>,
    child: Arc<Mutex<Option<Child>>>,
    closing: Arc<AtomicBool>,
    pid: Arc<AtomicU32>,
    confinement: Arc<Mutex<Value>>,
    memory: Arc<Mutex<Value>>,
    max_source: usize,
    ids: AtomicU64,
    queued: Arc<AtomicUsize>,
    max_queue: usize,
    cancellation: Cancellation,
}
impl Drop for StoreInner {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.closing.store(true, Ordering::Release);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}
#[derive(Clone)]
pub struct StoreHelper {
    inner: Arc<StoreInner>,
}
enum ParentEvent {
    Command(StoreCommand),
    Frame(Result<Value>, mpsc::Sender<()>),
}
struct QueuePermit(Arc<AtomicUsize>);
impl Drop for QueuePermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
struct StoreCommand {
    _permit: QueuePermit,
    id: u64,
    source: String,
    reply: SharedCompletion<Evaluation>,
}
impl StoreHelper {
    pub fn eval(&self, source: impl Into<String>) -> Result<Ticket<Evaluation>> {
        if self.inner.closing.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        let source = source.into();
        if source.len() > self.inner.max_source {
            return Err(Error::Quota("source bytes"));
        }
        self.inner
            .queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(1).filter(|n| *n <= self.inner.max_queue)
            })
            .map_err(|_| Error::Busy)?;
        let permit = QueuePermit(self.inner.queued.clone());
        let id = self.inner.ids.fetch_add(1, Ordering::Relaxed);
        let (ticket, reply) = ticket(id);
        self.inner
            .tx
            .try_send(ParentEvent::Command(StoreCommand {
                id,
                source,
                reply,
                _permit: permit,
            }))
            .map_err(|e| match e {
                mpsc::TrySendError::Full(_) => Error::Busy,
                _ => Error::Closed,
            })?;
        Ok(ticket)
    }
    pub fn status(&self) -> StoreStatus {
        StoreStatus {
            pid: self.inner.pid.load(Ordering::Acquire),
            closed: self.inner.closing.load(Ordering::Acquire),
            confinement: self.inner.confinement.lock().unwrap().clone(),
            memory: self.inner.memory.lock().unwrap().clone(),
            resident_bytes: resident_bytes(self.inner.pid.load(Ordering::Acquire)),
        }
    }
    /// Nonblocking process stop; the transport worker reaps and revokes leases.
    pub fn close(&self) {
        self.inner.cancellation.cancel();
        self.inner.closing.store(true, Ordering::Release);
        if let Some(child) = self.inner.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}
pub struct StorePool {
    executable: PathBuf,
    scope: String,
    config: Config,
    helpers: Mutex<HashMap<String, Weak<StoreInner>>>,
}
impl StorePool {
    pub fn new(
        executable: impl AsRef<Path>,
        scope: impl Into<String>,
        config: Config,
    ) -> Result<Self> {
        // Reuse the production runtime's limit validation without starting a realm.
        RuntimePool::new("validate-store", config.clone())?;
        if config.limits.max_source_bytes > FRAME_LIMIT / 8
            || config.limits.max_response_bytes > FRAME_LIMIT / 8
        {
            return Err(Error::Invalid("Store limits exceed IPC envelope"));
        }
        Ok(Self {
            executable: executable.as_ref().to_owned(),
            scope: scope.into(),
            config,
            helpers: Default::default(),
        })
    }
    pub fn helper(&self, cluster: &str) -> Result<StoreHelper> {
        if cluster.is_empty() || cluster.len() > 1024 {
            return Err(Error::Invalid("Store cluster id"));
        }
        let mut helpers = self.helpers.lock().unwrap();
        helpers.retain(|_, helper| helper.strong_count() > 0);
        if let Some(inner) = helpers
            .get(cluster)
            .and_then(Weak::upgrade)
            .filter(|i| !i.closing.load(Ordering::Acquire))
        {
            return Ok(StoreHelper { inner });
        }
        let (tx, rx) = mpsc::sync_channel(self.config.limits.queue_capacity);
        let inner = Arc::new(StoreInner {
            tx,
            child: Default::default(),
            closing: Default::default(),
            pid: Default::default(),
            confinement: Default::default(),
            memory: Default::default(),
            max_source: self.config.limits.max_source_bytes,
            ids: AtomicU64::new(1),
            queued: Default::default(),
            max_queue: self.config.limits.queue_capacity,
            cancellation: Cancellation::default(),
        });
        let worker = StoreWorker {
            executable: self.executable.clone(),
            scope: self.scope.clone(),
            cluster: cluster.to_owned(),
            config: self.config.clone(),
            events_tx: inner.tx.clone(),
            child: inner.child.clone(),
            closing: inner.closing.clone(),
            pid: inner.pid.clone(),
            confinement: inner.confinement.clone(),
            measurement: inner.memory.clone(),
            cancellation: inner.cancellation.clone(),
        };
        thread::Builder::new()
            .name("pm-store-ipc".into())
            .spawn(move || store_worker(rx, worker))
            .map_err(io_error)?;
        helpers.insert(cluster.to_owned(), Arc::downgrade(&inner));
        Ok(StoreHelper { inner })
    }
}
struct StoreWorker {
    executable: PathBuf,
    scope: String,
    cluster: String,
    config: Config,
    events_tx: SyncSender<ParentEvent>,
    child: Arc<Mutex<Option<Child>>>,
    closing: Arc<AtomicBool>,
    pid: Arc<AtomicU32>,
    confinement: Arc<Mutex<Value>>,
    measurement: Arc<Mutex<Value>>,
    cancellation: Cancellation,
}
fn store_worker(rx: mpsc::Receiver<ParentEvent>, worker: StoreWorker) {
    let StoreWorker {
        executable,
        scope,
        cluster,
        config,
        events_tx,
        child,
        closing,
        pid,
        confinement,
        measurement,
        cancellation,
    } = worker;
    static SESSION: AtomicU64 = AtomicU64::new(1);
    let session = SESSION.fetch_add(1, Ordering::Relaxed);
    let remote_realm = format!("{scope}:{:?}", RealmKind::Store(cluster.clone()));
    let mut leases = HashMap::new();
    let run = (|| -> Result<()> {
        let mut process = Command::new(executable)
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(io_error)?;
        pid.store(process.id(), Ordering::Release);
        let input = Arc::new(Mutex::new(process.stdin.take().unwrap()));
        let mut output = process.stdout.take().unwrap();
        *child.lock().unwrap() = Some(process);
        if closing.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        write_frame(
            &mut *input.lock().unwrap(),
            &json!({"v":PROTOCOL,"kind":"init","scope":scope,"cluster":cluster,"limits":limits_wire(&config.limits)}),
        )?;
        let ready = read_frame(&mut output)?;
        if ready["kind"] != "ready" {
            return Err(Error::Invalid("Store handshake"));
        }
        *confinement.lock().unwrap() = ready["confinement"].clone();
        thread::spawn(move || {
            loop {
                let frame = read_frame(&mut output);
                let failed = frame.is_err();
                let (ack_tx, ack_rx) = mpsc::channel();
                if events_tx.send(ParentEvent::Frame(frame, ack_tx)).is_err() || failed {
                    break;
                }
                if ack_rx.recv().is_err() {
                    break;
                }
            }
        });
        let mut pending: Option<StoreCommand> = None;
        let mut queue = VecDeque::<StoreCommand>::new();
        let mut hosts = HostJobs::default();
        let transport = (|| -> Result<()> {
            while !closing.load(Ordering::Acquire) {
                if pending.is_none()
                    && let Some(command) = queue.pop_front()
                {
                    pending = Some(command);
                    let command = pending.as_ref().unwrap();
                    write_frame(
                        &mut *input.lock().unwrap(),
                        &json!({"v":PROTOCOL,"kind":"eval","id":command.id,"source":command.source}),
                    )?;
                }
                let frame = match rx.recv().map_err(io_error)? {
                    ParentEvent::Command(command) => {
                        queue.push_back(command);
                        continue;
                    }
                    ParentEvent::Frame(frame, ack) => {
                        let _ = ack.send(());
                        frame?
                    }
                };
                match frame["kind"].as_str() {
                    Some("rpc") => {
                        let id = number(&frame, "id")?;
                        let value = dispatch(
                            &frame,
                            &remote_realm,
                            session,
                            &config,
                            &mut leases,
                            &mut hosts,
                            &cancellation,
                        );
                        write_frame(&mut *input.lock().unwrap(), &response(id, value))?;
                    }
                    Some("result") => {
                        let command = pending
                            .as_ref()
                            .ok_or(Error::Invalid("Unexpected Store result"))?;
                        if number(&frame, "id")? != command.id {
                            return Err(Error::Invalid("Store result id"));
                        }
                        *measurement.lock().unwrap() = frame["memory"].clone();
                        let result = wire_result(&frame).and_then(|value| {
                            let mut g = generation(&value["generation"])?;
                            validate_generation(&mut g, &remote_realm, session)?;
                            Ok(Evaluation {
                                generation: g,
                                value: string(&value, "text")?.into(),
                                cache_hits: number(&value, "cache_hits")? as usize,
                            })
                        });
                        complete(pending.take().unwrap().reply, result);
                    }
                    _ => return Err(Error::Invalid("Store IPC response kind")),
                }
            }
            Ok(())
        })();
        if let Some(command) = pending {
            complete(
                command.reply,
                Err(transport.clone().err().unwrap_or(Error::Closed)),
            );
        }
        for command in queue {
            complete(
                command.reply,
                Err(transport.clone().err().unwrap_or(Error::Closed)),
            );
        }
        transport?;
        Ok(())
    })();
    cancellation.cancel();
    closing.store(true, Ordering::Release);
    if let Some(mut process) = child.lock().unwrap().take() {
        let _ = process.kill();
        let _ = process.wait();
    }
    for g in leases.values() {
        config.kernel.revoke_generation(g);
        config.kernel.seal_open_gestures(g);
    }
    for event in rx.try_iter() {
        if let ParentEvent::Command(command) = event {
            complete(
                command.reply,
                Err(run.clone().err().unwrap_or(Error::Closed)),
            );
        }
    }
}
fn validate_generation(g: &mut Generation, realm: &str, session: u64) -> Result<()> {
    if g.realm != realm || g.number == 0 || g.number > u32::MAX as u64 || session > u32::MAX as u64
    {
        return Err(Error::Invalid("Store generation fence"));
    }
    g.number |= session << 32;
    Ok(())
}
#[derive(Default)]
struct HostJobs(HashMap<u64, (Box<dyn HostRequest>, Cancellation)>);
impl Drop for HostJobs {
    fn drop(&mut self) {
        for (job, cancellation) in self.0.values_mut() {
            cancellation.cancel();
            job.cancel();
        }
    }
}
fn dispatch(
    frame: &Value,
    realm: &str,
    session: u64,
    config: &Config,
    leases: &mut HashMap<u64, Generation>,
    hosts: &mut HostJobs,
    cancellation: &Cancellation,
) -> Result<Value> {
    let args = &frame["args"];
    let mut g = if args["generation"].is_null() {
        None
    } else {
        let mut g = generation(&args["generation"])?;
        validate_generation(&mut g, realm, session)?;
        Some(g)
    };
    match string(frame, "method")? {
        "load" => {
            if args["realm"] != realm {
                return Err(Error::Invalid("Store storage namespace"));
            }
            config.kernel.load_storage(realm)
        }
        "commit" => {
            let g = g.take().ok_or(Error::Invalid("Store generation"))?;
            let previous = if args["previous"].is_null() {
                None
            } else {
                let mut p = generation(&args["previous"])?;
                validate_generation(&mut p, realm, session)?;
                Some(p)
            };
            let ops = args["ops"].as_array().ok_or(Error::Invalid("Store ops"))?;
            if ops.len() > config.limits.max_ops
                || args["ops"].to_string().len() > config.limits.max_op_bytes
            {
                return Err(Error::Quota("Store ops"));
            }
            config
                .kernel
                .commit_activation(previous.as_ref(), &g, ops, &args["storage"])?;
            leases.insert(g.number, g);
            Ok(Value::Null)
        }
        "revoke" | "seal" => {
            let g = g.ok_or(Error::Invalid("Store generation"))?;
            if frame["method"] == "revoke" {
                config.kernel.revoke_generation(&g);
            } else {
                config.kernel.seal_open_gestures(&g);
                leases.remove(&g.number);
            }
            Ok(Value::Null)
        }
        "console" => {
            config.host.console(
                &g.ok_or(Error::Invalid("Store generation"))?,
                string(args, "level")?,
                string(args, "message")?,
            );
            Ok(Value::Null)
        }
        "engine" => config
            .engine
            .as_ref()
            .ok_or(Error::Invalid("Engine proxy unavailable"))?
            .call(
                &g.ok_or(Error::Invalid("Store generation"))?,
                string(args, "operation")?,
                &args["input"],
                HostBudget {
                    cancellation: cancellation.clone(),
                    deadline: Instant::now() + config.limits.host_timeout,
                    max_response_bytes: config.limits.max_response_bytes,
                },
            ),
        "fetch.start" => {
            if hosts.0.len() >= config.limits.max_host_ops {
                return Err(Error::Quota("Store host requests"));
            }
            let id = number(args, "request")?;
            if hosts.0.contains_key(&id) {
                return Err(Error::Invalid("Duplicate Store request"));
            }
            let cancellation = Cancellation::default();
            let request = FetchRequest {
                url: string(args, "url")?.into(),
                method: string(args, "method")?.into(),
                headers: serde_json::from_value(args["headers"].clone()).map_err(io_error)?,
                body: serde_json::from_value(args["body"].clone()).map_err(io_error)?,
                max_response_bytes: config.limits.max_response_bytes,
                deadline: Instant::now() + config.limits.host_timeout,
            };
            let url = url::Url::parse(&request.url).map_err(io_error)?;
            if !matches!(url.scheme(), "http" | "https")
                || request.body.len() > config.limits.max_response_bytes
            {
                return Err(Error::Invalid("Store fetch bounds"));
            }
            let job = config.host.fetch(
                &g.ok_or(Error::Invalid("Store generation"))?,
                request,
                cancellation.clone(),
            )?;
            hosts.0.insert(id, (job, cancellation));
            Ok(Value::Null)
        }
        "fetch.poll" => {
            let id = number(args, "request")?;
            let job = hosts
                .0
                .get_mut(&id)
                .ok_or(Error::Invalid("Unknown Store fetch"))?;
            if let Some(result) = job.0.poll() {
                hosts.0.remove(&id);
                let result = result?;
                if result.body.len() > config.limits.max_response_bytes {
                    return Err(Error::Quota("Store response bytes"));
                }
                Ok(
                    json!({"status":result.status,"url":result.url,"headers":result.headers,"body":result.body}),
                )
            } else {
                Ok(Value::Null)
            }
        }
        "fetch.cancel" => {
            if let Some((mut job, cancellation)) = hosts.0.remove(&number(args, "request")?) {
                cancellation.cancel();
                job.cancel();
            }
            Ok(Value::Null)
        }
        _ => Err(Error::Invalid("Store host operation")),
    }
}
fn limits_wire(l: &Limits) -> Value {
    json!({"heap":l.heap_bytes,"external":l.external_bytes,"source":l.max_source_bytes,"host_ops":l.max_host_ops,"op_bytes":l.max_op_bytes,"ops":l.max_ops,"response":l.max_response_bytes,"engine_response":l.max_engine_response_bytes,"timeout_ms":l.host_timeout.as_millis() as u64,"queue":l.queue_capacity})
}
fn limits_read(v: &Value) -> Result<Limits> {
    let response = number(v, "response")? as usize;
    // Absent from a peer that predates it: engine replies keep the response cap.
    let engine_response = match v.get("engine_response") {
        Some(_) => number(v, "engine_response")? as usize,
        None => response,
    };
    Ok(Limits {
        heap_bytes: number(v, "heap")? as usize,
        // A helper has its own cage: it takes its whole ceiling at start.
        initial_heap_bytes: number(v, "heap")? as usize,
        external_bytes: number(v, "external")? as usize,
        max_source_bytes: number(v, "source")? as usize,
        max_host_ops: number(v, "host_ops")? as usize,
        max_op_bytes: number(v, "op_bytes")? as usize,
        max_ops: number(v, "ops")? as usize,
        max_response_bytes: response,
        max_engine_response_bytes: engine_response,
        host_timeout: Duration::from_millis(number(v, "timeout_ms")?),
        queue_capacity: number(v, "queue")? as usize,
        // Store helpers stay contained: a fetch past their budget fails.
        queue_fetches: false,
    })
}

struct Rpc {
    id: u64,
    method: &'static str,
    args: Value,
    reply: mpsc::Sender<Result<Value>>,
}
enum ChildEvent {
    Frame(Result<Value>),
    Rpc(Rpc),
}
#[derive(Clone)]
struct Bridge {
    tx: mpsc::Sender<ChildEvent>,
    ids: Arc<AtomicU64>,
    timeout: Duration,
}
impl Bridge {
    fn begin(&self, method: &'static str, args: Value) -> Result<mpsc::Receiver<Result<Value>>> {
        let (tx, rx) = mpsc::channel();
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        self.tx
            .send(ChildEvent::Rpc(Rpc {
                id,
                method,
                args,
                reply: tx,
            }))
            .map_err(io_error)?;
        Ok(rx)
    }
    fn call(&self, method: &'static str, args: Value) -> Result<Value> {
        self.begin(method, args)?
            .recv_timeout(self.timeout)
            .map_err(io_error)?
    }
}
impl KernelHooks for Bridge {
    fn load_storage(&self, realm: &str) -> Result<Value> {
        self.call("load", json!({"realm":realm}))
    }
    fn commit_activation(
        &self,
        previous: Option<&Generation>,
        next: &Generation,
        ops: &[Value],
        storage: &Value,
    ) -> Result<()> {
        self.call("commit",json!({"generation":wire_generation(next),"previous":previous.map(wire_generation),"ops":ops,"storage":storage})).map(|_|())
    }
    fn commit_retained(&self, _: &Generation, _: &[Value]) -> Result<()> {
        Err(Error::Invalid(
            "Store retained callback needs an explicit lease",
        ))
    }
    fn revoke_generation(&self, g: &Generation) {
        let _ = self.call("revoke", json!({"generation":wire_generation(g)}));
    }
    fn seal_open_gestures(&self, g: &Generation) {
        let _ = self.call("seal", json!({"generation":wire_generation(g)}));
    }
}
impl HostProxy for Bridge {
    fn fetch(
        &self,
        g: &Generation,
        request: FetchRequest,
        cancellation: Cancellation,
    ) -> Result<Box<dyn HostRequest>> {
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        let pending=self.begin("fetch.start",json!({"generation":wire_generation(g),"request":id,"url":request.url,"method":request.method,"headers":request.headers,"body":request.body}))?;
        Ok(Box::new(RemoteRequest {
            bridge: self.clone(),
            id,
            pending: Some(pending),
            started: false,
            cancelled: false,
            cancellation,
            deadline: request.deadline,
        }))
    }
    fn console(&self, g: &Generation, level: &str, message: &str) {
        let _ = self.call(
            "console",
            json!({"generation":wire_generation(g),"level":level,"message":message}),
        );
    }
}
struct RemoteRequest {
    bridge: Bridge,
    id: u64,
    pending: Option<mpsc::Receiver<Result<Value>>>,
    started: bool,
    cancelled: bool,
    cancellation: Cancellation,
    deadline: Instant,
}
impl HostRequest for RemoteRequest {
    fn poll(&mut self) -> Option<Result<FetchResponse>> {
        if self.cancelled || self.cancellation.is_cancelled() || Instant::now() >= self.deadline {
            self.cancel();
            return Some(Err(Error::Terminated));
        }
        if let Some(pending) = &self.pending {
            match pending.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    let value = match result {
                        Ok(v) => v,
                        Err(e) => return Some(Err(e)),
                    };
                    if self.started && !value.is_null() {
                        return Some((|| {
                            Ok(FetchResponse {
                                status: number(&value, "status")? as u16,
                                url: string(&value, "url")?.into(),
                                headers: serde_json::from_value(value["headers"].clone())
                                    .map_err(io_error)?,
                                body: serde_json::from_value(value["body"].clone())
                                    .map_err(io_error)?,
                            })
                        })());
                    }
                    self.started = true;
                }
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(_) => return Some(Err(Error::Closed)),
            }
        }
        match self.bridge.begin("fetch.poll", json!({"request":self.id})) {
            Ok(rx) => self.pending = Some(rx),
            Err(e) => return Some(Err(e)),
        };
        None
    }
    fn cancel(&mut self) {
        if !self.cancelled {
            self.cancelled = true;
            self.cancellation.cancel();
            let _ = self
                .bridge
                .begin("fetch.cancel", json!({"request":self.id}));
        }
    }
}
impl EngineProxy for Bridge {
    fn call(&self, g: &Generation, op: &str, input: &Value, budget: HostBudget) -> Result<Value> {
        budget.check()?;
        self.call(
            "engine",
            json!({"generation":wire_generation(g),"operation":op,"input":input}),
        )
    }
}
struct StoreWatch;
impl WatchdogHooks for StoreWatch {
    fn notice(&self, _: &Generation, _: u64) {}
    fn prompt(&self, _: &Generation, _: u64) -> WatchdogDecision {
        WatchdogDecision::Stop
    }
}

/// Entry point for the signed Store executable. The sandbox trait permits an
/// explicit Linux double without pretending macOS APIs ran in Linux.
pub fn run_store_helper(sandbox: &dyn StoreSandbox) -> Result<()> {
    let mut input = std::io::stdin();
    let init = read_frame(&mut input)?;
    if init["kind"] != "init" {
        return Err(Error::Invalid("Store init"));
    }
    let limits = limits_read(&init["limits"])?;
    let cluster = string(&init, "cluster")?.to_owned();
    let scope = string(&init, "scope")?.to_owned();
    crate::vm::snapshot(SnapshotKind::Store);
    let confinement = sandbox.confine()?;
    let (tx, rx) = mpsc::channel();
    let bridge = Arc::new(Bridge {
        tx: tx.clone(),
        ids: Arc::new(AtomicU64::new(1)),
        timeout: limits.host_timeout,
    });
    let config = Config {
        limits: limits.clone(),
        watchdog: WatchdogPolicy {
            notice: Duration::from_secs(2),
            prompt: limits.host_timeout.max(Duration::from_secs(2)),
            repeat: Duration::from_secs(8),
        },
        kernel: bridge.clone(),
        host: bridge.clone(),
        watchdog_hooks: Arc::new(StoreWatch),
        cache: CodeCache::new(4 * 1024 * 1024),
        engine: Some(bridge),
        admission: Admission::new(limits.heap_bytes * 2),
    };
    let pool = RuntimePool::new(scope, config)?;
    let realm = pool.start_realm(RealmKind::Store(cluster))?;
    thread::spawn(move || {
        loop {
            let frame = read_frame(&mut input);
            let failed = frame.is_err();
            if tx.send(ChildEvent::Frame(frame)).is_err() || failed {
                break;
            }
        }
    });
    let mut output = std::io::stdout();
    write_frame(
        &mut output,
        &json!({"v":PROTOCOL,"kind":"ready","confinement":confinement}),
    )?;
    let mut pending: Option<(u64, Ticket<Evaluation>)> = None;
    let mut rpcs = HashMap::new();
    let mut measuring: Option<(u64, Result<Evaluation>, Ticket<Vec<GenerationMemory>>)> = None;
    loop {
        if let Some((id, ticket)) = &pending
            && let Some(result) = ticket.try_take()
        {
            measuring = Some((*id, result, realm.measure()?));
            pending = None;
        }
        if let Some((id, result, ticket)) = &mut measuring
            && let Some(stats) = ticket.try_take()
        {
            let result = std::mem::replace(result, Err(Error::Closed));
            let mut frame=response(*id,result.map(|v|json!({"generation":wire_generation(&v.generation),"text":v.value,"cache_hits":v.cache_hits})));
            frame["kind"] = json!("result");
            frame["memory"] = stats?
                .first()
                .map(|s| memory(&s.memory))
                .unwrap_or(Value::Null);
            write_frame(&mut output, &frame)?;
            measuring = None;
        }
        let event = if pending.is_some() || measuring.is_some() {
            match rx.recv_timeout(Duration::from_millis(2)) {
                Ok(e) => e,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => break,
            }
        } else {
            match rx.recv() {
                Ok(e) => e,
                Err(_) => break,
            }
        };
        match event {
            ChildEvent::Rpc(call) => {
                write_frame(
                    &mut output,
                    &json!({"v":PROTOCOL,"kind":"rpc","id":call.id,"method":call.method,"args":call.args}),
                )?;
                rpcs.insert(call.id, call.reply);
            }
            ChildEvent::Frame(frame) => {
                let frame = frame?;
                match frame["kind"].as_str() {
                    Some("reply") => {
                        let id = number(&frame, "id")?;
                        let reply = rpcs
                            .remove(&id)
                            .ok_or(Error::Invalid("Unknown Store RPC id"))?;
                        let _ = reply.send(wire_result(&frame));
                    }
                    Some("eval") if pending.is_none() && measuring.is_none() => {
                        pending = Some((
                            number(&frame, "id")?,
                            realm.eval(string(&frame, "source")?)?,
                        ));
                    }
                    _ => return Err(Error::Invalid("Store request kind")),
                }
            }
        }
    }
    realm.close();
    Ok(())
}
#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
