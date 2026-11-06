//! The application's extension host, one per project window. A dedicated owner
//! thread talks to V8; the main thread only enqueues and reads published copies,
//! so a slow, hung or failing extension can never stall AppKit (N8).
use crate::{
    adapter::KernelAdapter,
    compile,
    discovery::{self, Discovered},
    recorder::{CanvasText, NativeHost, RealmEvents},
    transport::{Transport, TransportOwner},
};
use crate::store_host::StoreExtension;
use pm_compat_host::{AdmittedDomBatch, CanvasFrame, RemoteDomPanel};
use pm_kernel::{
    KernelEvent, KernelHandle, RealmEnvelope, RealmMessage, Writer,
    state::{Registration, StateOp, StateTransaction},
};
use pm_remote_dom::{TrySink, protocol::Session};
use pm_runtime_v8::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

/// Every host gets its own kernel writer; one window's retirement is its own.
static NEXT_REALM: AtomicU32 = AtomicU32::new(1000);
const PANEL: u64 = 1;
const GENERATION: u32 = 1;
/// A realm that fails this often in one session stays off until reopened.
const MAX_RESTARTS: usize = 2;

pub struct HostConfig {
    pub kernel: KernelHandle,
    pub project: String,
    /// Extension folders; later roots replace earlier ones.
    pub roots: Vec<PathBuf>,
    /// Compiled module graphs, keyed by source digest.
    pub cache: PathBuf,
    pub owner: Option<Box<dyn TransportOwner>>,
    /// Canvas text against the app's font service, built on the owner thread.
    pub text: Option<Box<dyn Fn() -> CanvasText + Send>>,
    /// Published state changed. Runs on any thread; must only schedule.
    pub wake: Arc<dyn Fn() + Send + Sync>,
}

enum Input {
    Kernel,
    Realm(RealmEnvelope),
    Moved(bool, bool),
    Run(String, Value),
    Panel(String, bool),
    Place(String, [f32; 4]),
    Page(Value),
    Dom,
    Enabled(String, bool),
    Rescan,
    Failed,
    Close,
}

/// A contribution list is the realm's plus every confined helper's.
fn merge_store(value: &mut Value, store: &Value) {
    for (kind, items) in store.as_object().into_iter().flatten() {
        if let (Some(list), Some(items)) = (
            value[kind.as_str()].as_array_mut(),
            items.as_array(),
        ) {
            list.extend(items.iter().cloned());
        } else if value[kind.as_str()].is_null() {
            value[kind.as_str()] = items.clone();
        }
    }
}

#[derive(Default)]
struct Published {
    contributions: Option<Arc<Value>>,
    records: BTreeMap<String, Value>,
    toasts: Vec<String>,
    batches: VecDeque<AdmittedDomBatch>,
    canvases: Vec<CanvasFrame>,
    surfaces: Vec<Value>,
    ready: bool,
    revision: u64,
}

struct Shared {
    published: Mutex<Published>,
    /// What the confined helpers registered, in contribution-list form.
    store: Mutex<Value>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl Shared {
    fn update(&self, change: impl FnOnce(&mut Published)) {
        {
            let mut published = self.published.lock().unwrap();
            change(&mut published);
            published.revision += 1;
        }
        (self.wake)();
    }
}

/// Native surfaces read these registries: Effects, Properties, MCP and the kernel's
/// layer commands see a contribution exactly as they see a built-in.
fn registrations(value: &Value, owner: Writer) -> Vec<StateOp> {
    let mut ops = vec![StateOp::DisposeOwner { owner }];
    for (kind, registry) in [
        ("effects", "effects"),
        ("transitions", "transitions"),
        ("layers", "layerTypes"),
        ("panels", "panels"),
        ("commands", "commands"),
        ("inspector", "inspectorSections"),
        ("themes", "themes"),
        ("status", "status"),
    ] {
        for item in value[kind].as_array().into_iter().flatten() {
            let Some(id) = item["id"].as_str() else {
                continue;
            };
            let Ok(definition) = pm_model::json::parse(&item.to_string()) else {
                continue;
            };
            ops.push(StateOp::Register {
                registry: registry.into(),
                entry: Registration {
                    token: format!("{}.{}.{registry}.{id}", owner.realm, owner.generation),
                    owner,
                    id: id.into(),
                    definition,
                },
            });
        }
    }
    ops
}

struct Events {
    shared: Arc<Shared>,
    kernel: KernelHandle,
    writer: Writer,
    inbox: Sender<Input>,
    /// The extension whose activation is on the stack, for failure attribution.
    activating: Mutex<Option<String>>,
    dom_pending: AtomicBool,
}
impl RealmEvents for Arc<Events> {
    fn health(&self, id: &str, health: &Value) {
        let (id, health) = (id.to_owned(), health.clone());
        self.shared.update(|p| {
            if let Some(record) = p.records.get_mut(&id) {
                record["health"] = health;
            }
        });
    }
    fn toast(&self, text: &str, _: &Value) {
        let text = text.to_owned();
        self.shared.update(|p| p.toasts.push(text));
    }
    fn contributions(&self, mut value: Value) {
        let _ = self.kernel.state_transaction(StateTransaction {
            expected_revision: None,
            ops: registrations(&value, self.writer),
        });
        merge_store(&mut value, &self.shared.store.lock().unwrap());
        self.shared
            .update(|p| p.contributions = Some(Arc::new(value)));
    }
    fn dom_ready(&self) {
        if !self.dom_pending.swap(true, Ordering::AcqRel) {
            let _ = self.inbox.send(Input::Dom);
        }
    }
    fn ready(&self) {
        self.shared.update(|p| p.ready = true);
    }
    fn activating(&self, id: Option<&str>) {
        *self.activating.lock().unwrap() = id.map(str::to_owned);
    }
    fn watchdog_stop(&self) {
        let _ = self.inbox.send(Input::Failed);
    }
}

/// Bounded handoff to AppKit: a full queue is backpressure, never a dropped batch.
struct PageQueue<'a>(&'a Shared);
impl TrySink<AdmittedDomBatch> for PageQueue<'_> {
    fn try_send(&mut self, batch: AdmittedDomBatch) -> std::result::Result<(), pm_remote_dom::Error> {
        let mut published = self.0.published.lock().unwrap();
        if published.batches.len() >= 256 {
            return Err(pm_remote_dom::Error::Backpressure);
        }
        published.batches.push_back(batch);
        Ok(())
    }
}

pub struct ExtensionHost {
    inbox: Sender<Input>,
    shared: Arc<Shared>,
    transport: Arc<Mutex<Option<Arc<NativeHost>>>>,
    moved: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl ExtensionHost {
    pub fn start(config: HostConfig) -> Self {
        let (inbox, input) = mpsc::channel();
        let shared = Arc::new(Shared {
            published: Mutex::default(),
            store: Mutex::new(json!({})),
            wake: config.wake.clone(),
        });
        let transport = Arc::new(Mutex::new(None));
        let moved = Arc::new(AtomicBool::new(false));
        let writer = Writer {
            realm: NEXT_REALM.fetch_add(1, Ordering::Relaxed),
            generation: GENERATION,
        };
        forward_kernel(&config.kernel, inbox.clone());
        forward_mailbox(&config.kernel, writer, inbox.clone());
        let owner = Owner {
            writer,
            stores: vec![],
            shared: shared.clone(),
            inbox: inbox.clone(),
            transport: transport.clone(),
            moved: moved.clone(),
            restarts: 0,
            realm: None,
            router: None,
            records: vec![],
            canvases: BTreeMap::new(),
            disk: 0,
            config,
        };
        let worker = thread::Builder::new()
            .name("pm-compat-host".into())
            .spawn(move || owner.run(input))
            .expect("compat host thread");
        Self {
            inbox,
            shared,
            transport,
            moved,
            worker: Some(worker),
        }
    }
    /// Latest contributions, or `None` until the first activation pass finishes.
    pub fn contributions(&self) -> Option<Arc<Value>> {
        self.shared.published.lock().unwrap().contributions.clone()
    }
    pub fn revision(&self) -> u64 {
        self.shared.published.lock().unwrap().revision
    }
    pub fn is_ready(&self) -> bool {
        self.shared.published.lock().unwrap().ready
    }
    pub fn records(&self) -> Vec<Value> {
        let published = self.shared.published.lock().unwrap();
        published.records.values().cloned().collect()
    }
    /// Enabled v4 surface extensions with their compiled graphs.
    pub fn surfaces(&self) -> Vec<Value> {
        self.shared.published.lock().unwrap().surfaces.clone()
    }
    pub fn take_toasts(&self) -> Vec<String> {
        std::mem::take(&mut self.shared.published.lock().unwrap().toasts)
    }
    /// Admitted batches for `AdmittedDomBatch::present`, oldest first. Keep the
    /// front of a failed presentation and call `return_batches`.
    pub fn take_batches(&self) -> VecDeque<AdmittedDomBatch> {
        std::mem::take(&mut self.shared.published.lock().unwrap().batches)
    }
    pub fn return_batches(&self, mut rest: VecDeque<AdmittedDomBatch>) {
        let mut published = self.shared.published.lock().unwrap();
        rest.append(&mut published.batches);
        published.batches = rest;
    }
    pub fn take_canvases(&self) -> Vec<CanvasFrame> {
        std::mem::take(&mut self.shared.published.lock().unwrap().canvases)
    }
    pub fn run_command(&self, id: &str, args: Value) {
        let _ = self.inbox.send(Input::Run(id.to_owned(), args));
    }
    pub fn set_panel(&self, id: &str, open: bool) {
        let _ = self.inbox.send(Input::Panel(id.to_owned(), open));
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) {
        let _ = self.inbox.send(Input::Enabled(id.to_owned(), enabled));
    }
    pub fn rescan(&self) {
        let _ = self.inbox.send(Input::Rescan);
    }
    /// Page feedback. Called from AppKit: only enqueues.
    pub fn page_message(&self, message: Value) {
        let _ = self.inbox.send(Input::Page(message));
    }
    /// A sendable handle to `page_message` for a forwarding thread.
    pub fn page_sink(&self) -> impl Fn(Value) + Send + 'static + use<> {
        let inbox = self.inbox.clone();
        move |message| {
            let _ = inbox.send(Input::Page(message));
        }
    }
    /// Window position of a contributed panel's root, in points.
    pub fn place_panel(&self, id: &str, rect: [f32; 4]) {
        let _ = self.inbox.send(Input::Place(id.to_owned(), rect));
    }
    /// The app's clock moved. Cheap enough for every presented frame.
    pub fn publish_time(&self, time: f64, playing: bool) {
        if let Some(host) = &*self.transport.lock().unwrap() {
            let (time_moved, play_moved) = host.transport().publish(time, playing);
            if (time_moved || play_moved) && !self.moved.swap(true, Ordering::AcqRel) {
                let _ = self.inbox.send(Input::Moved(time_moved, play_moved));
            }
        }
    }
    pub fn close(&mut self) {
        let _ = self.inbox.send(Input::Close);
        if let Some(worker) = self.worker.take() {
            // The owner stops V8 and exits; joined off the main thread.
            thread::spawn(move || {
                let _ = worker.join();
            });
        }
    }
}
impl Drop for ExtensionHost {
    fn drop(&mut self) {
        self.close();
    }
}

fn forward_kernel(kernel: &KernelHandle, inbox: Sender<Input>) {
    let events = kernel.subscribe();
    thread::spawn(move || {
        while let Ok(event) = events.recv() {
            let relevant = matches!(
                event,
                KernelEvent::Published { .. }
                    | KernelEvent::SelectionChanged { .. }
                    | KernelEvent::WorkspaceChanged { .. }
                    | KernelEvent::StateChanged { .. }
            );
            if relevant && inbox.send(Input::Kernel).is_err() {
                break;
            }
        }
    });
}
fn forward_mailbox(kernel: &KernelHandle, writer: Writer, inbox: Sender<Input>) {
    let mailbox = kernel.register_realm(writer);
    thread::spawn(move || {
        while let Ok(envelope) = mailbox.recv() {
            if inbox.send(Input::Realm(envelope)).is_err() {
                break;
            }
        }
    });
}

struct Live {
    realm: Realm,
    pool: RuntimePool,
    host: Arc<NativeHost>,
    events: Arc<Events>,
}
struct Owner {
    writer: Writer,
    stores: Vec<(Writer, StoreExtension)>,
    config: HostConfig,
    shared: Arc<Shared>,
    inbox: Sender<Input>,
    transport: Arc<Mutex<Option<Arc<NativeHost>>>>,
    moved: Arc<AtomicBool>,
    restarts: usize,
    realm: Option<Live>,
    router: Option<RemoteDomPanel>,
    records: Vec<Value>,
    canvases: BTreeMap<(u64, String), u64>,
    disk: u64,
}
impl Owner {
    fn run(mut self, input: Receiver<Input>) {
        self.disk = self.stamp();
        self.boot();
        loop {
            let message = match input.recv_timeout(Duration::from_millis(1000)) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    // Extensions change on disk: an agent's edit, a manual copy.
                    let stamp = self.stamp();
                    if stamp != self.disk {
                        self.disk = stamp;
                        self.rescan();
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            // A burst of kernel publications is one mirror update.
            let mut kernel = matches!(message, Input::Kernel);
            let mut next = (!kernel).then_some(message);
            while next.is_none() {
                match input.try_recv() {
                    Ok(Input::Kernel) => kernel = true,
                    Ok(other) => next = Some(other),
                    Err(_) => break,
                }
            }
            if kernel {
                self.eval("__compat.sync()");
                self.follow_state();
            }
            match next {
                Some(Input::Close) => break,
                Some(message) => self.handle(message),
                None => (),
            }
        }
        if let Some(live) = self.realm.take() {
            live.realm.close();
        }
        *self.transport.lock().unwrap() = None;
        let _ = self.config.kernel.state_transaction(StateTransaction {
            expected_revision: None,
            ops: vec![StateOp::DisposeOwner { owner: self.writer }],
        });
    }
    fn eval(&mut self, source: &str) {
        let Some(live) = &self.realm else { return };
        match live.realm.eval(source) {
            Ok(_) => (),
            Err(Error::Busy) => {
                // The owner's queue is full: wait for room rather than drop state.
                let realm = live.realm.clone();
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(1));
                    if !matches!(realm.eval(source), Err(Error::Busy)) {
                        return;
                    }
                }
            }
            Err(_) => {
                let _ = self.inbox.send(Input::Failed);
            }
        }
    }
    fn handle(&mut self, message: Input) {
        match message {
            Input::Run(id, args) => {
                if !self.run_store(&id, &args) {
                    let source = format!("__compat.run({},{args})", json!(id));
                    self.eval(&source);
                }
            }
            Input::Panel(id, open) => {
                let source = format!(
                    "__compat.{}({})",
                    if open { "openPanel" } else { "closePanel" },
                    json!(id)
                );
                self.eval(&source);
            }
            Input::Place(id, rect) => {
                self.eval(&format!("__compat.placePanel({},{})", json!(id), json!(rect)));
            }
            Input::Moved(time, playing) => {
                self.moved.store(false, Ordering::Release);
                if let Some(live) = &self.realm {
                    let now = live.host.transport().time();
                    self.eval(&format!("__compat.moved({now},{time},{playing})"));
                }
            }
            Input::Realm(envelope) => self.realm_message(envelope),
            Input::Page(message) => self.page(message),
            Input::Dom => self.drain_dom(),
            Input::Enabled(id, enabled) => self.set_enabled(&id, enabled),
            Input::Rescan => self.rescan(),
            Input::Failed => self.restart(),
            Input::Kernel | Input::Close => (),
        }
    }
    /// A confined helper owns this command: its callback is a handle in there.
    fn run_store(&self, id: &str, args: &Value) -> bool {
        let state = self.config.kernel.state();
        let Some(entry) = state.provider("commands", id) else {
            return false;
        };
        let Some((_, extension)) = self.stores.iter().find(|(owner, _)| *owner == entry.owner)
        else {
            return false;
        };
        if let Some(handle) = entry.definition.get("run").and_then(|run| run.as_f64()) {
            extension.run(handle as u64, args);
        }
        true
    }
    /// Publishes what a helper registered through the same path the realm uses.
    fn publish_store(&self) {
        let state = self.config.kernel.state();
        let mut store = json!({});
        let mut definitions = vec![];
        for (owner, extension) in &self.stores {
            let mut own = json!({"effects":[],"transitions":[],"layers":[]});
            for (kind, registry) in [
                ("commands", "commands"),
                ("effects", "effects"),
                ("transitions", "transitions"),
                ("layers", "layerTypes"),
            ] {
                let items: Vec<Value> = state
                    .registries
                    .get(registry)
                    .into_iter()
                    .flatten()
                    .filter(|entry| entry.owner == *owner)
                    .filter_map(|entry| {
                        serde_json::from_str(&pm_model::json::to_canonical_string(
                            &entry.definition,
                        ))
                        .ok()
                    })
                    .collect();
                own[kind] = json!(items);
                let list = store[kind].as_array().cloned().unwrap_or_default();
                store[kind] = json!(list.into_iter().chain(items).collect::<Vec<_>>());
            }
            definitions.extend(extension.definitions(&own));
        }
        store["definitions"] = Value::Array(definitions);
        let mut published = self.shared.published.lock().unwrap();
        if let Some(current) = published.contributions.take() {
            let mut next = (*current).clone();
            let previous = self.shared.store.lock().unwrap().clone();
            for (kind, items) in previous.as_object().into_iter().flatten() {
                if let (Some(list), Some(items)) =
                    (next[kind.as_str()].as_array_mut(), items.as_array())
                {
                    list.retain(|item| !items.contains(item));
                }
            }
            merge_store(&mut next, &store);
            published.contributions = Some(Arc::new(next));
        }
        drop(published);
        *self.shared.store.lock().unwrap() = store;
        self.shared.update(|_| ());
    }
    fn realm_message(&mut self, envelope: RealmEnvelope) {
        if envelope.project != self.config.project {
            return;
        }
        let Some(live) = &self.realm else { return };
        let source = match envelope.message {
            RealmMessage::RunExternal { continuation, .. } => live
                .host
                .continuation(&continuation)
                .ok()
                .map(|reply| format!("__compat.runExternal({})", reply["continuation"])),
            RealmMessage::Cleanup { handle, .. } => live
                .host
                .handle_name(handle)
                .map(|name| format!("__compat.retire({})", json!(name))),
        };
        if let Some(source) = source {
            self.eval(&source);
        }
    }
    fn page(&mut self, message: Value) {
        let Some(router) = &mut self.router else { return };
        struct Realmward(Vec<Value>);
        impl TrySink<pm_remote_dom::protocol::PageMessage> for Realmward {
            fn try_send(
                &mut self,
                message: pm_remote_dom::protocol::PageMessage,
            ) -> std::result::Result<(), pm_remote_dom::Error> {
                self.0.push(
                    serde_json::to_value(message)
                        .map_err(|_| pm_remote_dom::Error::Invalid("page message"))?,
                );
                Ok(())
            }
        }
        let mut delivered = Realmward(vec![]);
        if router.receive(message, &mut delivered).is_err() {
            return;
        }
        for message in delivered.0 {
            self.eval(&format!(
                "__compat.dom.receive({message});__compat.dom.flush()"
            ));
        }
        self.drain_dom();
    }
    fn drain_dom(&mut self) {
        let (Some(live), Some(router)) = (&self.realm, &mut self.router) else {
            return;
        };
        live.events.dom_pending.store(false, Ordering::Release);
        for batch in live.host.take_dom_batches() {
            // A rejected tree is an extension fault; the page keeps its last state.
            let mut queue = PageQueue(&self.shared);
            let mut remaining = 200;
            loop {
                match router.enqueue(batch.clone(), &mut queue) {
                    Err(pm_compat_host::Error::Backpressure) if remaining > 0 => {
                        remaining -= 1;
                        thread::sleep(Duration::from_millis(5));
                    }
                    _ => break,
                }
            }
        }
        let _ = crate::settled(&live.realm, "__compat.flushCanvases?.()", 5);
        let frames: Vec<_> = live
            .host
            .canvas_frames()
            .unwrap_or_default()
            .into_iter()
            .filter(|frame| {
                let key = (frame.node, frame.surface.clone());
                self.canvases.insert(key, frame.version) != Some(frame.version)
            })
            .collect();
        if frames.is_empty() {
            (self.shared.wake)();
        } else {
            self.shared.update(|p| p.canvases.extend(frames));
        }
    }
    fn set_enabled(&mut self, id: &str, enabled: bool) {
        let state = self.config.kernel.state();
        if let Some(entry) = state.extensions.get(id)
            && entry.enabled != enabled
        {
            let _ = self.config.kernel.state_transaction(StateTransaction {
                expected_revision: None,
                ops: vec![StateOp::Extension(pm_kernel::state::ExtensionEntry {
                    enabled,
                    ..entry.clone()
                })],
            });
        }
        for record in &mut self.records {
            if record["id"] == id {
                record["enabled"] = json!(enabled);
                if enabled {
                    record["health"] = json!({"state":"ok"});
                }
            }
        }
        let records = self.trusted_records();
        self.eval(&format!(
            "__compat.update({records},{},\"enable\")",
            json!([id])
        ));
    }
    fn enabled(&self, id: &str) -> bool {
        let state = self.config.kernel.state();
        state.extensions.get(id).is_none_or(|entry| entry.enabled)
    }
    /// Settings and MCP read extensions from kernel state, as they do built-ins.
    fn publish_entries(&self) {
        let state = self.config.kernel.state();
        let ops: Vec<StateOp> = self
            .records
            .iter()
            .filter_map(|record| {
                let id = record["id"].as_str()?;
                let previous = state.extensions.get(id);
                let mut source = previous
                    .and_then(|entry| entry.source.as_object())
                    .cloned()
                    .unwrap_or_default();
                let summary = json!({"name":record["manifest"]["name"],"about":record["manifest"]["description"],
                    "health":record["health"],"trust":record["trust"],"contributes":record["manifest"]["contributes"]});
                source.insert("hostRecord", pm_model::json::parse(&summary.to_string()).ok()?);
                let next = pm_kernel::state::ExtensionEntry {
                    id: id.to_owned(),
                    installed: true,
                    enabled: previous.is_none_or(|entry| entry.enabled),
                    version: record["manifest"]["version"].as_str().unwrap_or("").to_owned(),
                    source: pm_model::Value::Object(source),
                };
                (previous != Some(&next)).then_some(StateOp::Extension(next))
            })
            .collect();
        if !ops.is_empty() {
            let _ = self.config.kernel.state_transaction(StateTransaction {
                expected_revision: None,
                ops,
            });
        }
    }
    /// A Settings toggle or an agent's edit lands in state or on disk; follow it.
    fn follow_state(&mut self) {
        let state = self.config.kernel.state();
        let toggled: Vec<(String, bool)> = self
            .records
            .iter()
            .filter_map(|record| {
                let id = record["id"].as_str()?;
                let enabled = state.extensions.get(id)?.enabled;
                (record["enabled"] != enabled).then(|| (id.to_owned(), enabled))
            })
            .collect();
        for (id, enabled) in toggled {
            self.set_enabled(&id, enabled);
        }
    }
    fn stamp(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for found in discovery::scan(&self.config.roots) {
            found.id.hash(&mut hash);
            if let Ok(tree) = compile::fingerprint(&found.directory) {
                tree.hash(&mut hash);
            }
        }
        hash.finish()
    }
    fn compile(&mut self, compiler: &SvelteCompiler, found: &Discovered) -> Value {
        let enabled = self.enabled(&found.id);
        let graph = match &found.manifest {
            Ok(manifest) => compile::compile(
                &found.directory,
                manifest["entry"].as_str().unwrap_or("index.ts"),
                manifest["apiVersion"].as_u64().unwrap_or(1),
                compiler,
                Some(&self.config.cache),
            ),
            Err(error) => Err(error.clone()),
        };
        let mut record = discovery::record(found, enabled, graph.as_ref().map(|g| g.to_json()).map_err(Clone::clone));
        // Surface extensions get their own realm from the panel host.
        if record["manifest"]["apiVersion"].as_u64() >= Some(4) {
            record["surface"] = json!(true);
            return record;
        }
        if record["trust"] != "store" || record["health"]["state"] != "ok" || !enabled {
            return record;
        }
        // Someone else's code: confined, and never in the plan of the trusted realm.
        record["confined"] = json!(true);
        let wants_full_access = record["manifest"]["permissions"]
            .as_array()
            .is_some_and(|p| p.iter().any(|p| p == "full-access"));
        if wants_full_access {
            record["health"] = json!({"state":"needs-trust"});
            return record;
        }
        let owner = Writer {
            realm: NEXT_REALM.fetch_add(1, Ordering::Relaxed),
            generation: GENERATION,
        };
        let started = graph.map_err(Error::Host).and_then(|graph| {
            StoreExtension::start(
                Arc::new(self.config.kernel.clone()),
                &self.config.project,
                "native",
                found,
                &graph,
                found.store.as_ref().unwrap_or(&Value::Null),
                owner,
            )
        });
        match started {
            Ok(extension) => self.stores.push((owner, extension)),
            Err(error) => {
                record["health"] = json!({"state":"activation-error","error":error.to_string()});
            }
        }
        record
    }
    fn publish_records(&self) {
        let records: BTreeMap<_, _> = self
            .records
            .iter()
            .map(|record| {
                let mut row = record.clone();
                if let Some(row) = row.as_object_mut() {
                    row.remove("graph");
                }
                (record["id"].as_str().unwrap_or_default().to_owned(), row)
            })
            .collect();
        let surfaces: Vec<Value> = self
            .records
            .iter()
            .filter(|r| r["surface"] == true && r["health"]["state"] == "ok" && r["enabled"] == true)
            .cloned()
            .collect();
        self.shared.update(|p| {
            p.records = records;
            p.surfaces = surfaces;
        });
        self.publish_entries();
    }
    fn start_realm(&mut self) -> Result<()> {
        let events = Arc::new(Events {
            shared: self.shared.clone(),
            kernel: self.config.kernel.clone(),
            writer: self.writer,
            inbox: self.inbox.clone(),
            activating: Mutex::default(),
            dom_pending: AtomicBool::new(false),
        });
        let adapter = KernelAdapter::new(
            self.config.kernel.clone(),
            &self.config.project,
            crate::PROJECT,
            self.writer.realm,
        );
        let transport = Transport::new(self.config.owner.take());
        let host = Arc::new(NativeHost::new(adapter, transport, Box::new(events.clone())));
        if let Some(text) = &self.config.text {
            host.set_text(text());
        }
        let pool = crate::pool(&format!("window:{}", self.config.project), host.clone())?;
        let realm = pool.realm(RealmKind::Compat)?;
        crate::boot_realm(&realm, &host, "")?;
        *self.transport.lock().unwrap() = Some(host.clone());
        self.router = Some(
            RemoteDomPanel::new(
                PANEL,
                Session {
                    panel: "compat".into(),
                    generation: GENERATION,
                },
            )
            .map_err(|e| Error::Host(format!("{e:?}")))?,
        );
        self.realm = Some(Live {
            realm,
            pool,
            host,
            events,
        });
        Ok(())
    }
    fn boot(&mut self) {
        let found = discovery::scan(&self.config.roots);
        if found.is_empty() {
            // Nothing to run: no realm, no memory, no bundle evaluation.
            self.shared.update(|p| p.ready = true);
            return;
        }
        if let Err(error) = self.start_realm() {
            self.shared
                .update(|p| p.toasts.push(format!("Extensions could not start: {error}")));
            return;
        }
        let live = self.realm.as_ref().unwrap();
        let compiler = match SvelteCompiler::new(&live.pool, 256 * 1024 * 1024, 8 * 1024 * 1024) {
            Ok(compiler) => compiler,
            Err(error) => {
                self.shared
                    .update(|p| p.toasts.push(format!("Extensions could not compile: {error}")));
                return;
            }
        };
        self.records = found.iter().map(|f| self.compile(&compiler, f)).collect();
        compiler.realm().close();
        self.publish_records();
        self.publish_store();
        let records = self.trusted_records();
        self.eval(&format!("__compat.boot({records})"));
    }
    /// The plan of the trusted realm. Confined records never enter it.
    fn trusted_records(&self) -> Value {
        Value::Array(
            self.records
                .iter()
                .filter(|record| record["confined"] != true && record["surface"] != true)
                .cloned()
                .collect(),
        )
    }
    fn rescan(&mut self) {
        if self.realm.is_none() {
            self.boot();
            return;
        }
        let found = discovery::scan(&self.config.roots);
        let live = self.realm.as_ref().unwrap();
        let Ok(compiler) = SvelteCompiler::new(&live.pool, 256 * 1024 * 1024, 8 * 1024 * 1024)
        else {
            return;
        };
        self.stores.clear();
        let records: Vec<_> = found.iter().map(|f| self.compile(&compiler, f)).collect();
        compiler.realm().close();
        let changed: Vec<_> = records
            .iter()
            .filter(|next| {
                !self
                    .records
                    .iter()
                    .any(|old| old["id"] == next["id"] && old["graph"]["hash"] == next["graph"]["hash"])
            })
            .map(|record| record["id"].clone())
            .collect();
        self.records = records;
        self.publish_records();
        self.publish_store();
        let records = self.trusted_records();
        self.eval(&format!(
            "__compat.update({records},{},\"rescan\")",
            Value::Array(changed)
        ));
    }
    /// A hung or failed realm is rebuilt without the extension that broke it.
    /// The kernel already sealed its edits; other windows and the UI never noticed.
    fn restart(&mut self) {
        let Some(live) = self.realm.take() else { return };
        let culprit = live.events.activating.lock().unwrap().take();
        live.realm.close();
        drop(live);
        self.restarts += 1;
        if let Some(id) = &culprit {
            for record in &mut self.records {
                if record["id"] == id.as_str() {
                    record["enabled"] = json!(false);
                    record["health"] = json!({"state":"runtime-error","error":"Stopped responding"});
                }
            }
            let id = id.clone();
            self.shared
                .update(|p| p.toasts.push(format!("{id} stopped working — check Mods")));
        }
        if self.restarts > MAX_RESTARTS {
            self.shared.update(|p| {
                p.toasts
                    .push("Extensions stopped after repeated failures. Reopen the project to retry.".into());
            });
            return;
        }
        if self.start_realm().is_ok() {
            self.publish_records();
            let records = self.trusted_records();
            self.eval(&format!("__compat.boot({records})"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::{HistoryCall, KernelActor, KernelConfig, OpenSource, Reply, Request};
    use std::{fs, path::Path};
    fn extension(root: &Path, id: &str, files: &[(&str, &str)]) {
        let directory = root.join(id);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("manifest.json"),
            json!({"id":id,"name":id,"version":"1.0.0","apiVersion":3}).to_string(),
        )
        .unwrap();
        for (name, source) in files {
            fs::write(directory.join(name), source).unwrap();
        }
    }
    struct Fixture {
        actor: Option<KernelActor>,
        kernel: KernelHandle,
        host: ExtensionHost,
        _root: tempfile::TempDir,
    }
    impl Fixture {
        fn start(root: tempfile::TempDir) -> Self {
            let actor = KernelActor::spawn(KernelConfig::default());
            let kernel = actor.handle();
            let document = pm_model::json::parse(&crate::initial()["document"].to_string()).unwrap();
            kernel
                .open("window", OpenSource::Untitled(document))
                .wait()
                .unwrap()
                .unwrap();
            let host = ExtensionHost::start(HostConfig {
                kernel: kernel.clone(),
                project: "window".into(),
                roots: vec![root.path().join("extensions")],
                cache: root.path().join("cache"),
                owner: None,
                text: None,
                wake: Arc::new(|| ()),
            });
            Self {
                actor: Some(actor),
                kernel,
                host,
                _root: root,
            }
        }
        fn until(&self, what: &str, done: impl Fn(&ExtensionHost) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !done(&self.host) {
                assert!(
                    Instant::now() < deadline,
                    "{what}; toasts {:?}, records {:?}",
                    self.host.take_toasts(),
                    self.host.records()
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
        fn name(&self) -> String {
            let published = self.kernel.published("window").unwrap();
            let root = published.snapshot.root().unwrap();
            root.get("name")
                .and_then(|n| n.to_value().as_js_string().map(|s| s.to_string_lossy()))
                .unwrap_or_default()
        }
        fn health(&self, id: &str) -> String {
            self.host
                .records()
                .iter()
                .find(|r| r["id"] == id)
                .map(|r| r["health"]["state"].as_str().unwrap_or_default().to_owned())
                .unwrap_or_default()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.host.close();
            if let Some(worker) = self.host.worker.take() {
                let _ = worker.join();
            }
            if let Some(actor) = self.actor.take() {
                thread::sleep(Duration::from_millis(50));
                actor.shutdown();
            }
        }
    }
    fn command(fixture: &Fixture, id: &str) -> bool {
        fixture.host.contributions().is_some_and(|c| {
            c["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|command| command["id"] == id)
        })
    }
    #[test]
    fn discovered_typescript_and_svelte_extensions_contribute_and_edit_the_live_project() {
        let root = tempfile::tempdir().unwrap();
        extension(
            &root.path().join("extensions"),
            "synthetic-good",
            &[
                (
                    "index.ts",
                    "import { label } from './label';
                     import Panel from './Panel.svelte';
                     import type { Nothing } from './types';
                     export default function activate(api: any): void {
                       const unused: Nothing | null = null;
                       api.commands.register({id:'synthetic-good.rename',label:label(),run(name: string){
                         api.history.do('Rename',()=>{api.project.get().name=name});
                         return name}});
                       api.panels.register({id:'synthetic-good.panel',title:'Synthetic',component:Panel});
                     }",
                ),
                ("label.ts", "export const label = (): string => 'Rename project';"),
                ("types.ts", "export type Nothing = never;"),
                (
                    "Panel.svelte",
                    "<script lang='ts'>let { api }: { api: any } = $props();</script><p class='note'>{api.project.get().name}</p><style>.note{color:red}</style>",
                ),
            ],
        );
        let fixture = Fixture::start(root);
        fixture.until("activation", |host| host.is_ready());
        assert_eq!(fixture.health("synthetic-good"), "ok");
        assert!(command(&fixture, "synthetic-good.rename"));
        let contributions = fixture.host.contributions().unwrap();
        assert_eq!(contributions["panels"][0]["title"], "Synthetic");
        fixture
            .host
            .run_command("synthetic-good.rename", json!(["Renamed by extension"]));
        let deadline = Instant::now() + Duration::from_secs(5);
        while fixture.name() != "Renamed by extension" {
            assert!(Instant::now() < deadline, "edit reached the kernel");
            thread::sleep(Duration::from_millis(5));
        }
        // A native undo reaches the same mirror.
        let ack = fixture
            .kernel
            .call_blocking(
                "window",
                Request::new(pm_kernel::Caller::native(), HistoryCall::Undo),
            )
            .unwrap();
        assert!(matches!(ack.reply, Reply::Bool(true)));
        assert_eq!(fixture.name(), "Synthetic extension check");
    }
    #[test]
    fn contributed_effects_reach_kernel_registries_and_engine_definitions() {
        let root = tempfile::tempdir().unwrap();
        extension(
            &root.path().join("extensions"),
            "synthetic-effect",
            &[(
                "index.ts",
                "export default function(api){api.effects.register({id:'synthetic.tint',label:'Tint',group:'Stylize',params:[{k:'amount',label:'Amount',min:0,max:2,def:1},{k:'color',label:'Color',type:'color',def:'#ff0000'}],frag:'o = src(v_uv) * u_amount;'})}",
            )],
        );
        let fixture = Fixture::start(root);
        fixture.until("activation", |host| host.is_ready());
        let state = fixture.kernel.state();
        let provider = state.provider("effects", "synthetic.tint").unwrap();
        assert_eq!(provider.definition.get("label").unwrap().as_str(), Some("Tint"));
        let contributions = fixture.host.contributions().unwrap();
        let definition: pm_protocol::definitions::DefinitionSnapshot =
            serde_json::from_value(contributions["definitions"][0].clone()).unwrap();
        let glsl = definition.glsl.unwrap();
        assert!(glsl.source.starts_with("#version 300 es"));
        assert!(glsl.source.contains("uniform float u_amount;"));
        assert!(glsl.source.contains("uniform vec3 u_color;"));
        assert_eq!(
            glsl.source.lines().nth(glsl.body_line_offset as usize),
            Some("o = src(v_uv) * u_amount;")
        );
    }
    #[test]
    fn a_store_install_runs_confined_and_contributes_like_a_local_one() {
        let root = tempfile::tempdir().unwrap();
        let extensions = root.path().join("extensions");
        extension(
            &extensions,
            "synthetic-store",
            &[
                (
                    "index.ts",
                    "export default function(api: any){
                       api.effects.register({id:'tint',label:'Store tint',group:'Stylize',params:[{k:'amount',label:'Amount',min:0,max:1,def:1}],frag:'o = src(v_uv) * u_amount;'});
                       api.commands.register({id:'mark',label:'Mark',run(){globalThis.ran=(typeof document)+':'+(typeof PM)}});
                     }",
                ),
                (".powermove-store.json", "{\"repoId\":\"r\",\"releaseId\":\"synthetic\"}"),
            ],
        );
        let fixture = Fixture::start(root);
        fixture.until("activation", |host| {
            host.is_ready() && host.contributions().is_some()
        });
        assert_eq!(fixture.health("synthetic-store"), "ok", "{:?}", fixture.host.records());
        let state = fixture.kernel.state();
        assert!(state.provider("effects", "synthetic-store.tint").is_some());
        assert!(state.provider("commands", "synthetic-store.mark").is_some());
        let contributions = fixture.host.contributions().unwrap();
        let definitions = contributions["definitions"].as_array().unwrap();
        assert_eq!(definitions[0]["origin"], "store");
        assert_eq!(definitions[0]["id"], "synthetic-store.tint");
        assert!(command(&fixture, "synthetic-store.mark"));
    }
    #[test]
    fn a_v4_extension_compiles_as_a_surface_and_stays_out_of_the_trusted_realm() {
        let root = tempfile::tempdir().unwrap();
        let extensions = root.path().join("extensions");
        fs::create_dir_all(extensions.join("synthetic-v4")).unwrap();
        fs::write(
            extensions.join("synthetic-v4/manifest.json"),
            json!({"id":"synthetic-v4","name":"Synthetic surface","version":"1.0.0","apiVersion":4,"entry":"panel.tsx"}).to_string(),
        )
        .unwrap();
        fs::write(
            extensions.join("synthetic-v4/panel.tsx"),
            "import { useState } from 'powermove/hooks';
             export const view = () => <text>{String(useState(1)[0])}</text>;",
        )
        .unwrap();
        let fixture = Fixture::start(root);
        fixture.until("activation", |host| host.is_ready());
        let surfaces = fixture.host.surfaces();
        assert_eq!(surfaces.len(), 1);
        let graph = &surfaces[0]["graph"];
        let source = graph["sources"]["panel.tsx"].as_str().unwrap();
        assert!(source.contains("powermove/jsx-runtime"), "{source}");
        assert_eq!(
            graph["edges"]["panel.tsx"]["powermove/hooks"],
            "runtime:powermove/hooks"
        );
    }
    #[test]
    fn settings_toggles_and_disk_edits_reach_the_live_realm() {
        let root = tempfile::tempdir().unwrap();
        let extensions = root.path().join("extensions");
        let write = |label: &str| {
            extension(
                &extensions,
                "synthetic-live",
                &[(
                    "index.ts",
                    &format!(
                        "export default function(api){{api.commands.register({{id:'synthetic-live.run',label:'{label}',run(){{}}}})}}"
                    ),
                )],
            );
        };
        write("First");
        let fixture = Fixture::start(root);
        let label = |host: &ExtensionHost| {
            host.contributions()
                .and_then(|c| c["commands"][0]["label"].as_str().map(str::to_owned))
        };
        fixture.until("activation", |host| label(host).as_deref() == Some("First"));
        let state = fixture.kernel.state();
        let entry = state.extensions.get("synthetic-live").unwrap().clone();
        assert_eq!(entry.version, "1.0.0");
        fixture
            .kernel
            .state_transaction(StateTransaction {
                expected_revision: None,
                ops: vec![StateOp::Extension(pm_kernel::state::ExtensionEntry {
                    enabled: false,
                    ..entry.clone()
                })],
            })
            .wait()
            .unwrap()
            .unwrap();
        fixture.until("unload", |host| label(host).is_none());
        fixture
            .kernel
            .state_transaction(StateTransaction {
                expected_revision: None,
                ops: vec![StateOp::Extension(pm_kernel::state::ExtensionEntry {
                    enabled: true,
                    ..entry
                })],
            })
            .wait()
            .unwrap()
            .unwrap();
        fixture.until("reload", |host| label(host).as_deref() == Some("First"));
        std::thread::sleep(Duration::from_millis(1100));
        extension(
            &fixture._root.path().join("extensions"),
            "synthetic-live",
            &[(
                "index.ts",
                "export default function(api){api.commands.register({id:'synthetic-live.run',label:'Second',run(){}})}",
            )],
        );
        fixture.until("rescan", |host| label(host).as_deref() == Some("Second"));
    }
    #[test]
    fn one_bad_extension_leaves_the_others_running() {
        let root = tempfile::tempdir().unwrap();
        let extensions = root.path().join("extensions");
        extension(
            &extensions,
            "synthetic-throws",
            &[("index.ts", "export default function(){throw new Error('synthetic')}")],
        );
        extension(
            &extensions,
            "synthetic-broken",
            &[("index.ts", "export default function(api){ this is not javascript")],
        );
        extension(
            &extensions,
            "synthetic-missing",
            &[("index.ts", "import './nowhere'; export default function(){}")],
        );
        extension(
            &extensions,
            "synthetic-noexport",
            &[("index.ts", "export const nothing = 1;")],
        );
        extension(
            &extensions,
            "synthetic-good",
            &[(
                "index.ts",
                "export default function(api){api.commands.register({id:'synthetic-good.run',label:'Run',run(){}})}",
            )],
        );
        let fixture = Fixture::start(root);
        fixture.until("activation", |host| host.is_ready());
        assert_eq!(fixture.health("synthetic-good"), "ok");
        assert_eq!(fixture.health("synthetic-throws"), "activation-error");
        assert_eq!(fixture.health("synthetic-broken"), "build-error");
        assert_eq!(fixture.health("synthetic-missing"), "build-error");
        assert!(matches!(
            fixture.health("synthetic-noexport").as_str(),
            "build-error" | "activation-error"
        ));
        assert!(command(&fixture, "synthetic-good.run"));
    }
    #[test]
    fn a_hung_activation_is_stopped_and_the_realm_comes_back_without_it() {
        let root = tempfile::tempdir().unwrap();
        let extensions = root.path().join("extensions");
        extension(
            &extensions,
            "synthetic-hangs",
            &[("index.ts", "export default function(){for(;;);}")],
        );
        extension(
            &extensions,
            "synthetic-good",
            &[(
                "index.ts",
                "export default function(api){api.commands.register({id:'synthetic-good.run',label:'Run',run(){}})}",
            )],
        );
        let fixture = Fixture::start(root);
        fixture.until("recovery", |host| {
            let stopped = host
                .records()
                .iter()
                .any(|r| r["id"] == "synthetic-hangs" && r["health"]["state"] == "runtime-error");
            stopped
                && host.is_ready()
                && host.contributions().is_some_and(|c| {
                    c["commands"].as_array().unwrap().iter().any(|c| c["id"] == "synthetic-good.run")
                })
        });
        assert_eq!(fixture.health("synthetic-good"), "ok");
    }
}
