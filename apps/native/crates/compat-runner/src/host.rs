//! The application's extension host, one per project window. A dedicated owner
//! thread talks to V8; the main thread only enqueues and reads published copies,
//! so a slow, hung or failing extension can never stall AppKit (N8).
use crate::{
    adapter::KernelAdapter,
    compile,
    discovery::{self, Discovered},
    recorder::{NativeHost, RealmEvents},
    transport::{Transport, TransportOwner},
};
use pm_compat_host::{AdmittedDomBatch, CanvasFrame, RemoteDomPanel};
use pm_kernel::{KernelEvent, KernelHandle, RealmEnvelope, RealmMessage, Writer};
use pm_remote_dom::{TrySink, protocol::Session};
use pm_runtime_v8::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

const REALM_ID: u32 = 2;
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

#[derive(Default)]
struct Published {
    contributions: Option<Arc<Value>>,
    records: BTreeMap<String, Value>,
    toasts: Vec<String>,
    batches: VecDeque<AdmittedDomBatch>,
    canvases: Vec<CanvasFrame>,
    ready: bool,
    revision: u64,
}

struct Shared {
    published: Mutex<Published>,
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

struct Events {
    shared: Arc<Shared>,
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
    fn contributions(&self, value: Value) {
        self.shared.update(|p| p.contributions = Some(Arc::new(value)));
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
            wake: config.wake.clone(),
        });
        let transport = Arc::new(Mutex::new(None));
        let moved = Arc::new(AtomicBool::new(false));
        forward_kernel(&config.kernel, inbox.clone());
        forward_mailbox(&config.kernel, inbox.clone());
        let owner = Owner {
            shared: shared.clone(),
            inbox: inbox.clone(),
            transport: transport.clone(),
            moved: moved.clone(),
            restarts: 0,
            realm: None,
            router: None,
            records: vec![],
            canvases: BTreeMap::new(),
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
            );
            if relevant && inbox.send(Input::Kernel).is_err() {
                break;
            }
        }
    });
}
fn forward_mailbox(kernel: &KernelHandle, inbox: Sender<Input>) {
    let mailbox = kernel.register_realm(Writer {
        realm: REALM_ID,
        generation: GENERATION,
    });
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
}
impl Owner {
    fn run(mut self, input: Receiver<Input>) {
        self.boot();
        loop {
            let message = match input.recv_timeout(Duration::from_millis(250)) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => continue,
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
                let source = format!("__compat.run({},{args})", json!(id));
                self.eval(&source);
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
        let _ = self
            .config
            .kernel
            .state_transaction(pm_kernel::state::StateTransaction {
                expected_revision: None,
                ops: vec![pm_kernel::state::StateOp::Setting {
                    key: format!("extension.{id}.enabled"),
                    value: Some(pm_model::Value::Bool(enabled)),
                }],
            });
        for record in &mut self.records {
            if record["id"] == id {
                record["enabled"] = json!(enabled);
                if enabled {
                    record["health"] = json!({"state":"ok"});
                }
            }
        }
        let records = Value::Array(self.records.clone());
        self.eval(&format!(
            "__compat.update({records},{},\"enable\")",
            json!([id])
        ));
    }
    fn enabled(&self, id: &str) -> bool {
        let state = self.config.kernel.state();
        state
            .settings
            .get(&format!("extension.{id}.enabled"))
            .is_none_or(|value| *value != pm_model::Value::Bool(false))
    }
    fn compile(&self, compiler: &SvelteCompiler, found: &Discovered) -> Value {
        let enabled = self.enabled(&found.id);
        let graph = match &found.manifest {
            Ok(manifest) => compile::compile(
                &found.directory,
                manifest["entry"].as_str().unwrap_or("index.ts"),
                compiler,
                Some(&self.config.cache),
            )
            .map(|graph| graph.to_json()),
            Err(error) => Err(error.clone()),
        };
        discovery::record(found, enabled, graph)
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
        self.shared.update(|p| p.records = records);
    }
    fn start_realm(&mut self) -> Result<()> {
        let events = Arc::new(Events {
            shared: self.shared.clone(),
            inbox: self.inbox.clone(),
            activating: Mutex::default(),
            dom_pending: AtomicBool::new(false),
        });
        let adapter = KernelAdapter::new(
            self.config.kernel.clone(),
            &self.config.project,
            crate::PROJECT,
            REALM_ID,
        );
        let transport = Transport::new(self.config.owner.take());
        let host = Arc::new(NativeHost::new(adapter, transport, Box::new(events.clone())));
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
        let records = Value::Array(self.records.clone());
        self.eval(&format!("__compat.boot({records})"));
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
        let records = Value::Array(self.records.clone());
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
            let records = Value::Array(self.records.clone());
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
