//! The application's extension host, one per project window. A dedicated owner
//! thread talks to V8; the main thread only enqueues and reads published copies,
//! so a slow, hung or failing extension can never stall AppKit (N8).
//!
//! The kernel is app-wide. The realm is a writer generation: rebuilding it seals
//! the old generation and starts a new one, with a new page behind its panels.
use crate::{
    adapter::KernelAdapter,
    compile::{self, ModuleGraph},
    discovery::{self, Discovered},
    recorder::{CanvasText, NativeHost, RealmEvents, SyncLayout},
    store_kernel::update_state,
    transport::{Transport, TransportOwner},
};
use crate::{store_host::StoreExtension, store_install, store_runtime::StoreViews};
use pm_compat_host::{AdmittedDomBatch, CanvasFrame, RemoteDomPanel};
use pm_kernel::{
    KernelEvent, KernelHandle, RealmEnvelope, RealmMessage, Writer,
    state::{Registration, StateOp, StateTransaction},
};
use pm_remote_dom::{TrySink, protocol::Session};
use pm_runtime_v8::*;
use pm_store_host::adapter::StoreSession;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::Duration,
};

mod export;

/// Every host gets its own kernel writer; one window's retirement is its own.
static NEXT_REALM: AtomicU32 = AtomicU32::new(1000);
const PANEL: u64 = 1;
const GENERATION: u32 = 1;
/// The page document of a realm starts its own session, whatever the writer's
/// generation: a rebuilt realm always gets a new page.
const PAGE_SESSION: u32 = 1;
/// A realm that fails this often in one session stays off until reopened.
const MAX_RESTARTS: usize = 2;
/// Admitted batches waiting for a page. Past this, batches wait in the realm,
/// whose document coalesces them, so memory stays bounded with no page at all.
const PAGE_QUEUE: usize = 256;
/// How often an idle owner looks for extensions changed on disk.
const DISK_POLL: Duration = Duration::from_secs(3);
/// How long a synchronous layout read waits for the page, and for a page
/// that has not started reading yet.
const SYNC_LAYOUT_WAIT: Duration = Duration::from_millis(250);
const SYNC_LAYOUT_FIRST_WAIT: Duration = Duration::from_secs(3);
/// Realm sources the owner holds while the realm's queue is full.
const MAX_DEFERRED: usize = 4096;
const TIME_MOVED: u8 = 1;
const PLAY_MOVED: u8 = 2;

/// Every running host's inbox; a closed one drops out on the next send.
static HOSTS: Mutex<Vec<Sender<Input>>> = Mutex::new(Vec::new());
/// Extensions changed on disk (an agent published, Undo restored): every
/// window rescans now instead of at its next disk poll.
pub fn rescan_all() {
    HOSTS
        .lock()
        .unwrap()
        .retain(|inbox| inbox.send(Input::Rescan).is_ok());
}

pub struct HostConfig {
    pub kernel: KernelHandle,
    pub project: String,
    /// Extension folders; later roots replace earlier ones.
    pub roots: Vec<PathBuf>,
    /// Compiled module graphs, keyed by source digest.
    pub cache: PathBuf,
    /// The app's clock. Every realm generation asks the same owner.
    pub owner: Option<Arc<dyn TransportOwner>>,
    /// Canvas text against the app's font service, built on the owner thread.
    pub text: Option<Box<dyn Fn() -> CanvasText + Send>>,
    /// Published state changed. Runs on any thread; must only schedule.
    pub wake: Arc<dyn Fn() + Send + Sync>,
    /// The open composition rendered by the engine at a time and size, as PNG
    /// (`render.renderFrameTo`, `PM.Export.snapshot`).
    pub frames: Option<crate::FrameRenderer>,
    /// The app's base rasters and override answers (`render.raster`).
    pub rasters: Option<crate::RasterHooks>,
}

enum Input {
    Kernel,
    Viewport(f32, f32, f32),
    Realm(Writer, RealmEnvelope),
    Moved,
    /// The viewer presented a frame; the latest waits in `presented`.
    Presented,
    Run(String, Value),
    Panel(String, bool),
    Place(String, Option<[f32; 4]>),
    Page(u64, Value),
    Answer(u64, Value),
    /// A batch of layers the viewer or an export needs override answers for:
    /// `{needs, ask}`.
    Rasters(u64, Value),
    /// An export's first override: its own realm starts (`export.rs`).
    Export(
        String,
        Arc<pm_kernel::Published>,
        Arc<AtomicBool>,
        (Sender<export::RunInput>, Receiver<export::RunInput>),
    ),
    /// The shell's tool and the viewer's navigation, for built-in services.
    Shell(Value),
    Menu(String, Value, u64),
    StorePage(String, u64, Value),
    StoreDom(String),
    Dom,
    Canvases,
    Health(String, Value),
    Enabled(String, bool),
    Booted(u32),
    Rescan,
    Failed(u32),
    /// The watchdog's question about one running task, and the user's answer.
    Unresponsive(u32, u64),
    StopTask(u64, bool),
    /// The realm's page settled these faces (`family|weight|italic` keys).
    FontsLoaded(Value),
    Close,
}

/// A contribution list is the realm's plus every confined helper's.
fn merge_store(value: &mut Value, store: &Value) {
    for (kind, items) in store.as_object().into_iter().flatten() {
        if let (Some(list), Some(items)) = (value[kind.as_str()].as_array_mut(), items.as_array()) {
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
    /// Admitted view batches of each confined helper run, for its own page.
    store_batches: BTreeMap<(String, u64), VecDeque<AdmittedDomBatch>>,
    /// Questions only the main thread can answer: menus, sheets, panel reveals.
    requests: Vec<Value>,
    batches: VecDeque<AdmittedDomBatch>,
    /// The latest frame of each bound canvas; a newer version replaces it.
    canvases: BTreeMap<(u64, String), CanvasFrame>,
    surfaces: Vec<Value>,
    ready: bool,
    /// The realm generation drawing now, 0 with none. Its page is keyed by this.
    view: u64,
    revision: u64,
}

struct Shared {
    published: Mutex<Published>,
    /// What the confined helpers registered, in contribution-list form.
    store: Mutex<Value>,
    sessions: Mutex<BTreeMap<String, StoreSession>>,
    /// The generation whose page is reading batches. Canvases encode only for it.
    reader: AtomicU64,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// Running exports' realms and their pages.
    exports: Mutex<export::Exports>,
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
    /// Synchronous layout reads and the page's answers to them.
    measuring: Mutex<Measures>,
    /// A read already waited for a page that never started reading.
    page_missed: AtomicBool,
    dom_pending: AtomicBool,
    canvas_pending: AtomicBool,
}
impl RealmEvents for Arc<Events> {
    fn health(&self, id: &str, health: &Value) {
        let _ = self
            .inbox
            .send(Input::Health(id.to_owned(), health.clone()));
    }
    fn toast(&self, text: &str, _: &Value) {
        let text = text.to_owned();
        self.shared.update(|p| p.toasts.push(text));
    }
    fn request(&self, request: Value) {
        self.shared.update(|p| p.requests.push(request));
    }
    fn set_enabled(&self, id: &str, enabled: bool) {
        let _ = self.inbox.send(Input::Enabled(id.to_owned(), enabled));
    }
    fn contributions(&self, mut value: Value) {
        drop(self.kernel.state_transaction(StateTransaction {
            expected_revision: None,
            ops: registrations(&value, self.writer),
        }));
        merge_store(&mut value, &self.shared.store.lock().unwrap());
        self.shared
            .update(|p| p.contributions = Some(Arc::new(value)));
    }
    fn dom_ready(&self) {
        if !self.dom_pending.swap(true, Ordering::AcqRel) {
            let _ = self.inbox.send(Input::Dom);
        }
    }
    fn canvases_ready(&self) {
        if !self.canvas_pending.swap(true, Ordering::AcqRel) {
            let _ = self.inbox.send(Input::Canvases);
        }
    }
    fn ready(&self) {
        let _ = self.inbox.send(Input::Booted(self.writer.generation));
    }
    fn activating(&self, id: Option<&str>) {
        *self.activating.lock().unwrap() = id.map(str::to_owned);
    }
    fn unresponsive(&self, execution: u64) {
        let _ = self
            .inbox
            .send(Input::Unresponsive(self.writer.generation, execution));
    }
    /// The owner routes the page's answer here (`Owner::page`) while the
    /// realm waits; the main thread never waits for it.
    fn measure_sync(&self, request: u64) -> SyncLayout {
        let rx = match self.measuring.lock().unwrap().wait(request) {
            Ok(answer) => return SyncLayout::Answer(answer),
            Err(rx) => rx,
        };
        self.dom_ready();
        // A page still loading (the window just opened) answers once it
        // starts reading; a read answered with nothing would be remembered
        // by the extension as the layout.
        // Without a window (headless) no page ever comes: only one read waits.
        let reading =
            self.shared.reader.load(Ordering::Acquire) == u64::from(self.writer.generation);
        let missed = self.page_missed.load(Ordering::Acquire);
        let wait = match (reading, missed) {
            (false, true) => Duration::ZERO,
            (false, false) => SYNC_LAYOUT_FIRST_WAIT,
            (true, _) => SYNC_LAYOUT_WAIT,
        };
        let answer = rx.recv_timeout(wait);
        self.measuring.lock().unwrap().waiting = None;
        match answer {
            Ok(answer) => SyncLayout::Answer(answer),
            Err(_) if reading => SyncLayout::TimedOut,
            Err(_) => {
                self.page_missed.store(true, Ordering::Release);
                SyncLayout::NoPage
            }
        }
    }
}

/// The first extension entries this process publishes drop the last run's
/// D11 stops.
static FIRST_ENTRIES: AtomicBool = AtomicBool::new(true);
/// Whether D11 Stop turned the extension off for this run.
pub fn stopped_this_run(entry: &pm_kernel::state::ExtensionEntry) -> bool {
    entry
        .source
        .get("hostRecord")
        .and_then(|record| record.get("stopped"))
        .and_then(pm_model::Value::as_bool)
        == Some(true)
}
/// Ends a D11 stop on the entry (Settings, or an extension turning it on).
pub fn clear_stop(entry: &mut pm_kernel::state::ExtensionEntry) {
    if let Some(record) = entry
        .source
        .as_object_mut()
        .and_then(|source| source.get_mut("hostRecord"))
        .and_then(pm_model::Value::as_object_mut)
    {
        record.remove("stopped");
    }
}
/// Synchronous layout reads: the read waiting for the page, and answers that
/// came before their read started waiting (the request goes out with the
/// realm's batch first, so the page can answer before the wait begins).
#[derive(Default)]
struct Measures {
    waiting: Option<(u64, std::sync::mpsc::SyncSender<Value>)>,
    early: VecDeque<(u64, Value)>,
}
impl Measures {
    /// The answer, when it is already here; else where it will arrive.
    fn wait(
        &mut self,
        request: u64,
    ) -> std::result::Result<Value, std::sync::mpsc::Receiver<Value>> {
        if let Some(index) = self.early.iter().position(|(r, _)| *r == request) {
            return Ok(self.early.remove(index).unwrap().1);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.waiting = Some((request, tx));
        Err(rx)
    }
    fn answer(&mut self, request: u64, message: &Value) {
        match &self.waiting {
            Some((waiting, reply)) if *waiting == request => {
                let _ = reply.try_send(message.clone());
            }
            _ => {
                // Only the latest few: a read waits for its own, just sent.
                if self.early.len() >= 8 {
                    self.early.pop_front();
                }
                self.early.push_back((request, message.clone()));
            }
        }
    }
}

/// A confined helper's views: its batches wait here for the owner to admit them.
struct ViewEvents {
    id: String,
    shared: Arc<Shared>,
    inbox: Sender<Input>,
    pending: Mutex<VecDeque<Value>>,
    /// A batch was refused; the helper's document holds it until asked again.
    refused: AtomicBool,
}
impl StoreViews for ViewEvents {
    fn batch(&self, batch: Value) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if pending.len() >= 64 {
            self.refused.store(true, Ordering::Release);
            return false;
        }
        pending.push_back(batch);
        let _ = self.inbox.send(Input::StoreDom(self.id.clone()));
        true
    }
    fn panel(&self, id: &str, open: bool) {
        let request = json!({"kind":"panel","id":id,"open":open});
        self.shared.update(|p| p.requests.push(request));
    }
}

/// Bounded handoff to AppKit: a full queue is backpressure, never a dropped batch.
struct PageQueue<'a>(&'a Shared);
struct StorePageQueue<'a>(&'a Shared, (String, u64));
impl TrySink<AdmittedDomBatch> for StorePageQueue<'_> {
    fn try_send(
        &mut self,
        batch: AdmittedDomBatch,
    ) -> std::result::Result<(), pm_remote_dom::Error> {
        let mut published = self.0.published.lock().unwrap();
        let queue = published.store_batches.entry(self.1.clone()).or_default();
        if queue.len() >= PAGE_QUEUE {
            return Err(pm_remote_dom::Error::Backpressure);
        }
        queue.push_back(batch);
        drop(published);
        (self.0.wake)();
        Ok(())
    }
}
impl TrySink<AdmittedDomBatch> for PageQueue<'_> {
    fn try_send(
        &mut self,
        batch: AdmittedDomBatch,
    ) -> std::result::Result<(), pm_remote_dom::Error> {
        let mut published = self.0.published.lock().unwrap();
        if published.batches.len() >= PAGE_QUEUE {
            return Err(pm_remote_dom::Error::Backpressure);
        }
        published.batches.push_back(batch);
        Ok(())
    }
}

/// Override asks for one realm. While the realm waits for a capture of its own
/// project, they go to that wait (it answers inline); otherwise to its inbox.
#[derive(Clone)]
pub struct RasterInbox {
    inbox: Arc<Mutex<Sender<Input>>>,
    native: Arc<Mutex<Option<Arc<NativeHost>>>>,
    shared: Arc<Shared>,
}
impl RasterInbox {
    /// `needs`: `[{composition, layer, seconds, scale, crop?}]`; `ask`:
    /// `{revision}` (viewer) or `{job}` (an export started with
    /// [`Self::export`]). Answered through `RasterHooks::answers` with `batch`.
    pub fn ask(&self, batch: u64, needs: Value, ask: Value) {
        if let Some(job) = ask["job"].as_str() {
            let route = self.shared.exports.lock().unwrap().route(job);
            if let Some((inbox, capturing)) = route {
                // A capture its realm waits for: answered inline there.
                let wire = json!({"needs": needs, "ask": {}});
                let pending = match capturing {
                    Some(host) => host.nested_ask(batch, wire).err(),
                    None => Some(wire),
                };
                if let Some(wire) = pending {
                    let _ = inbox.send(export::RunInput::Ask(batch, wire["needs"].clone()));
                }
            } else if let Some(native) = self.native.lock().unwrap().clone() {
                // A capture the window's realm waits for (`export` kept no
                // route for it): answered inline there; otherwise the job is over.
                let _ = native.nested_ask(batch, json!({"needs": needs, "ask": {}}));
            }
            return;
        }
        let wire = json!({"needs": needs, "ask": ask});
        let native = self.native.lock().unwrap().clone();
        let wire = match native {
            Some(native) => match native.nested_ask(batch, wire) {
                Ok(()) => return,
                Err(wire) => wire,
            },
            None => wire,
        };
        let _ = self.inbox.lock().unwrap().send(Input::Rasters(batch, wire));
    }
    /// The export `job` froze `published`: its overrides are answered by a
    /// realm of its own (`export.rs`), started now, until `cancel` is set
    /// (the export ended) or the window's extensions stop.
    pub fn export(
        &self,
        job: &str,
        published: Arc<pm_kernel::Published>,
        cancel: Arc<AtomicBool>,
    ) -> std::result::Result<(), String> {
        // A capture the window's realm is waiting for, at its own revision
        // (`renderFrameTo`, a snapshot): that realm answers inline.
        let native = self.native.lock().unwrap().clone();
        if native.and_then(|n| n.capturing_at())
            == Some((published.snapshot.doc_id, published.revision()))
        {
            return Ok(());
        }
        let mut exports = self.shared.exports.lock().unwrap();
        let at = (published.snapshot.doc_id, published.revision());
        if exports.inbox(job).is_some() || exports.adopt_capture(job, at) {
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        exports.insert(
            job.to_owned(),
            export::Route {
                inbox: tx.clone(),
                view: export::next_view(),
                batches: VecDeque::new(),
                host: None,
            },
        );
        drop(exports);
        // The window makes the realm's page when it next looks.
        (self.shared.wake)();
        self.inbox
            .lock()
            .unwrap()
            .send(Input::Export(job.to_owned(), published, cancel, (tx, rx)))
            .map_err(|_| {
                self.shared.exports.lock().unwrap().remove(job);
                "The project's extensions stopped".to_owned()
            })
    }
}

pub struct ExtensionHost {
    inbox: Sender<Input>,
    shared: Arc<Shared>,
    kernel: KernelHandle,
    profile: PathBuf,
    transport: Arc<Mutex<Option<Arc<NativeHost>>>>,
    moved: Arc<AtomicU8>,
    presented: Arc<Mutex<Option<Value>>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl ExtensionHost {
    pub fn start(config: HostConfig) -> Self {
        let (inbox, input) = mpsc::channel();
        let kernel = config.kernel.clone();
        let profile = config
            .roots
            .last()
            .and_then(|root| root.parent())
            .unwrap_or(&config.cache)
            .to_path_buf();
        let shared = Arc::new(Shared {
            published: Mutex::default(),
            store: Mutex::new(json!({})),
            sessions: Mutex::default(),
            reader: AtomicU64::new(0),
            wake: config.wake.clone(),
            exports: Mutex::default(),
        });
        let transport = Arc::new(Mutex::new(None));
        let moved = Arc::new(AtomicU8::new(0));
        let presented = Arc::new(Mutex::new(None));
        let writer = Writer {
            realm: NEXT_REALM.fetch_add(1, Ordering::Relaxed),
            generation: GENERATION,
        };
        forward_kernel(&config.kernel, inbox.clone());
        forward_mailbox(&config.kernel, writer, inbox.clone());
        HOSTS.lock().unwrap().push(inbox.clone());
        let owner = Owner {
            writer,
            retired: false,
            stores: vec![],
            confined: BTreeMap::new(),
            open: BTreeMap::new(),
            viewport: None,
            shared: shared.clone(),
            inbox: inbox.clone(),
            transport: transport.clone(),
            moved: moved.clone(),
            presented: presented.clone(),
            shell: serde_json::Map::new(),
            restarts: 0,
            asking: None,
            asking_culprit: None,
            running: None,
            realm: None,
            router: None,
            records: vec![],
            canvases: BTreeMap::new(),
            dom_waiting: VecDeque::new(),
            deferred: VecDeque::new(),
            disk: 0,
            store_revision: None,
            closing: Arc::default(),
            config,
        };
        let worker = thread::Builder::new()
            .name("pm-compat-host".into())
            .spawn(move || owner.run(input))
            .expect("compat host thread");
        Self {
            inbox,
            shared,
            kernel,
            profile,
            transport,
            moved,
            presented,
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
    /// The realm generation drawing now, 0 with none. A page belongs to one
    /// generation: when this moves, that page is finished.
    pub fn view(&self) -> u64 {
        self.shared.published.lock().unwrap().view
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
    pub fn take_store_batches(&self, id: &str, view: u64) -> VecDeque<AdmittedDomBatch> {
        let mut published = self.shared.published.lock().unwrap();
        let taken = published
            .store_batches
            .remove(&(id.to_owned(), view))
            .unwrap_or_default();
        drop(published);
        // Room again: whatever was held back can move.
        if !taken.is_empty() {
            let _ = self.inbox.send(Input::StoreDom(id.to_owned()));
        }
        taken
    }
    pub fn return_store_batches(&self, id: &str, view: u64, mut rest: VecDeque<AdmittedDomBatch>) {
        let mut published = self.shared.published.lock().unwrap();
        let queue = published
            .store_batches
            .entry((id.to_owned(), view))
            .or_default();
        rest.append(queue);
        *queue = rest;
    }
    /// What a Store page needs: its data store and permissions.
    pub fn store_session(&self, id: &str) -> Option<StoreSession> {
        self.shared.sessions.lock().unwrap().get(id).cloned()
    }
    /// Feedback from the page of one helper run; a later run ignores it.
    pub fn store_page_sink(&self, id: &str, view: u64) -> impl Fn(Value) + Send + 'static + use<> {
        let (inbox, id) = (self.inbox.clone(), id.to_owned());
        move |message| {
            let _ = inbox.send(Input::StorePage(id.clone(), view, message));
        }
    }
    pub fn take_requests(&self) -> Vec<Value> {
        std::mem::take(&mut self.shared.published.lock().unwrap().requests)
    }
    pub fn answer(&self, token: u64, value: Value) {
        let _ = self.inbox.send(Input::Answer(token, value));
    }
    /// The answer to an `unresponsive` request: stop that task (the realm is
    /// rebuilt without the extension that was activating), or keep waiting.
    pub fn answer_unresponsive(&self, execution: u64, stop: bool) {
        let _ = self.inbox.send(Input::StopTask(execution, stop));
    }
    /// The Store's listings, as a request for the main thread to show.
    pub fn browse_store(&self) {
        let shared = self.shared.clone();
        thread::spawn(move || match store_install::catalog() {
            Ok(items) => shared.update(|p| p.requests.push(json!({"kind":"store","items":items}))),
            Err(error) => shared.update(|p| p.toasts.push(error.to_string())),
        });
    }
    /// Verifies and installs off every thread that matters, then discovers it.
    pub fn install_store(&self, release: &str, repo: &str) {
        let (shared, inbox) = (self.shared.clone(), self.inbox.clone());
        let (release, repo) = (release.to_owned(), repo.to_owned());
        let kernel = self.kernel.clone();
        let profile = self.profile.clone();
        thread::spawn(move || {
            match store_install::install(&kernel, &profile, "native", &release, &repo) {
                Ok(_) => {
                    let _ = inbox.send(Input::Rescan);
                }
                Err(error) => shared.update(|p| p.toasts.push(error.to_string())),
            }
        });
    }
    /// The shell's current tool (`{tool}`) or viewer navigation
    /// (`{zoom, pan, fit}`), which the `tool` and `viewer` services report.
    pub fn shell_state(&self, state: Value) {
        let _ = self.inbox.send(Input::Shell(state));
    }
    /// The viewer presented a frame (`{time, version, quality, viewport}`):
    /// the realm hears `frame:rendered`, then redraws `overlay`. Cheap enough
    /// for every frame; a busy realm hears only the latest.
    pub fn frame_presented(&self, frame: Value) {
        if self.presented.lock().unwrap().replace(frame).is_none() {
            let _ = self.inbox.send(Input::Presented);
        }
    }
    /// Where override asks for this realm go, from any thread.
    pub fn raster_inbox(&self) -> RasterInbox {
        RasterInbox {
            inbox: Arc::new(Mutex::new(self.inbox.clone())),
            native: self.transport.clone(),
            shared: self.shared.clone(),
        }
    }
    /// Running exports' realms (job, view): each gets a hidden page in the
    /// window for the layout reads its overrides make.
    pub fn export_pages(&self) -> Vec<(String, u64)> {
        self.shared.exports.lock().unwrap().pages()
    }
    pub fn take_export_batches(&self, job: &str) -> VecDeque<AdmittedDomBatch> {
        let mut exports = self.shared.exports.lock().unwrap();
        let taken = exports.batches(job).map(std::mem::take).unwrap_or_default();
        if let Some(inbox) = exports.inbox(job) {
            let _ = inbox.send(export::RunInput::Dom);
        }
        taken
    }
    pub fn return_export_batches(&self, job: &str, mut rest: VecDeque<AdmittedDomBatch>) {
        let mut exports = self.shared.exports.lock().unwrap();
        if let Some(queue) = exports.batches(job) {
            rest.append(queue);
            *queue = rest;
        }
    }
    /// The page under `key` (`""`: the window's realm; `export:<job>`: an
    /// export's) settled the faces `keys` (loaded, or failed to load).
    pub fn fonts_loaded(&self, key: &str, keys: Value) {
        match key.strip_prefix("export:") {
            None => {
                let _ = self.inbox.send(Input::FontsLoaded(keys));
            }
            Some(job) => {
                if let Some(inbox) = self.shared.exports.lock().unwrap().inbox(job) {
                    let _ = inbox.send(export::RunInput::FontsLoaded(keys));
                }
            }
        }
    }
    /// Feedback from an export realm's page, on a forwarding thread.
    pub fn export_page_sink(&self, job: &str) -> impl Fn(Value) + Send + 'static + use<> {
        let inbox = self.shared.exports.lock().unwrap().inbox(job);
        move |message| {
            if let Some(inbox) = &inbox {
                let _ = inbox.send(export::RunInput::Page(message));
            }
        }
    }
    /// One menu from several locations: `[[location, context], ...]`, gathered
    /// in order and answered once with every contribution.
    pub fn gather_menus(&self, pairs: Vec<Value>, key: u64) {
        let _ = self
            .inbox
            .send(Input::Menu(String::new(), Value::Array(pairs), key));
    }
    /// Contributions for one open menu. A `key` keeps the reply tied to it.
    pub fn gather_menu(&self, location: &str, context: Value, key: u64) {
        let _ = self
            .inbox
            .send(Input::Menu(location.to_owned(), context, key));
    }
    /// Admitted batches of generation `view` for `AdmittedDomBatch::present`,
    /// oldest first. Keep the front of a failed presentation and return the rest.
    pub fn take_batches(&self, view: u64) -> VecDeque<AdmittedDomBatch> {
        let mut published = self.shared.published.lock().unwrap();
        let taken = if published.view == view {
            std::mem::take(&mut published.batches)
        } else {
            VecDeque::new()
        };
        drop(published);
        // A first reader wants canvases; a drained queue has room for more.
        if self.shared.reader.swap(view, Ordering::AcqRel) != view || !taken.is_empty() {
            let _ = self.inbox.send(Input::Dom);
        }
        taken
    }
    pub fn return_batches(&self, view: u64, mut rest: VecDeque<AdmittedDomBatch>) {
        let mut published = self.shared.published.lock().unwrap();
        if published.view == view {
            rest.append(&mut published.batches);
            published.batches = rest;
        }
    }
    pub fn take_canvases(&self) -> Vec<CanvasFrame> {
        std::mem::take(&mut self.shared.published.lock().unwrap().canvases)
            .into_values()
            .collect()
    }
    pub fn run_command(&self, id: &str, args: Value) {
        let _ = self.inbox.send(Input::Run(id.to_owned(), args));
    }
    pub fn set_panel(&self, id: &str, open: bool) {
        let _ = self.inbox.send(Input::Panel(id.to_owned(), open));
    }
    /// Feedback from the page of generation `view`. Called on a forwarding
    /// thread; a later generation ignores it.
    pub fn page_sink(&self, view: u64) -> impl Fn(Value) + Send + 'static + use<> {
        let inbox = self.inbox.clone();
        move |message| {
            let _ = inbox.send(Input::Page(view, message));
        }
    }
    /// Window position of a contributed panel's root, in points. `None` is a
    /// tab behind another: open, kept, and neither painted nor hit.
    pub fn place_panel(&self, id: &str, rect: Option<[f32; 4]>) {
        let _ = self.inbox.send(Input::Place(id.to_owned(), rect));
    }
    /// The editor window's viewport, for v1–3's `window.innerWidth` & co.
    pub fn set_viewport(&self, width: f32, height: f32, scale: f32) {
        let _ = self.inbox.send(Input::Viewport(width, height, scale));
    }
    /// The app's clock moved. Cheap enough for every presented frame.
    pub fn publish_time(&self, time: f64, playing: bool) {
        if let Some(host) = &*self.transport.lock().unwrap() {
            let (time_moved, play_moved) = host.transport().publish(time, playing);
            let moved = (if time_moved { TIME_MOVED } else { 0 })
                | (if play_moved { PLAY_MOVED } else { 0 });
            // One message at a time; what it reports accumulates until it runs.
            if moved != 0 && self.moved.fetch_or(moved, Ordering::AcqRel) == 0 {
                let _ = self.inbox.send(Input::Moved);
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
/// Ends when the kernel retires this generation's mailbox.
fn forward_mailbox(kernel: &KernelHandle, writer: Writer, inbox: Sender<Input>) {
    let mailbox = kernel.register_realm(writer);
    thread::spawn(move || {
        while let Ok(envelope) = mailbox.recv() {
            if inbox.send(Input::Realm(writer, envelope)).is_err() {
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
struct StoreLive {
    owner: Writer,
    /// The source this helper runs: a rescan restarts it only when this moves.
    hash: String,
    extension: StoreExtension,
    views: Arc<ViewEvents>,
}
struct Owner {
    writer: Writer,
    /// This generation is sealed; the next realm starts a new one.
    retired: bool,
    stores: Vec<StoreLive>,
    /// What each Store extension would run, so a toggle can start it.
    confined: BTreeMap<String, (Discovered, ModuleGraph)>,
    /// Open panels and their last placement, replayed into a rebuilt realm or helper.
    open: BTreeMap<String, Option<[f32; 4]>>,
    /// The editor window's viewport, replayed into a rebuilt realm before boot.
    viewport: Option<(f32, f32, f32)>,
    config: HostConfig,
    shared: Arc<Shared>,
    inbox: Sender<Input>,
    transport: Arc<Mutex<Option<Arc<NativeHost>>>>,
    moved: Arc<AtomicU8>,
    presented: Arc<Mutex<Option<Value>>>,
    /// Every shell fact the services report, replayed into a rebuilt realm.
    shell: serde_json::Map<String, Value>,
    restarts: usize,
    /// The task the user is being asked about, so one hang asks once at a time.
    /// The task the D11 sheet asks about, and the extension running it.
    asking: Option<u64>,
    asking_culprit: Option<String>,
    /// The last command's task and the extension that registered it.
    running: Option<(u64, String)>,
    realm: Option<Live>,
    router: Option<RemoteDomPanel>,
    records: Vec<Value>,
    canvases: BTreeMap<(u64, String), u64>,
    /// Batches taken from the realm that the page queue has no room for yet.
    dom_waiting: VecDeque<Value>,
    /// Realm sources waiting for room in its queue (`eval`).
    deferred: VecDeque<String>,
    disk: u64,
    store_revision: Option<u64>,
    /// Set when this window's extensions stop; its export realms end too.
    closing: Arc<AtomicBool>,
}
impl Owner {
    fn run(mut self, input: Receiver<Input>) {
        self.disk = self.stamp();
        self.boot();
        loop {
            self.retry_deferred();
            let wait = if self.deferred.is_empty() {
                DISK_POLL
            } else {
                Duration::from_millis(2)
            };
            let message = match input.recv_timeout(wait) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) if !self.deferred.is_empty() => continue,
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
                self.tick_stores();
            }
            match next {
                Some(Input::Close) => break,
                Some(message) => self.handle(message),
                None => (),
            }
        }
        for store in std::mem::take(&mut self.stores) {
            self.stop_store(store);
        }
        self.closing.store(true, Ordering::Release);
        self.shared.exports.lock().unwrap().clear();
        self.retire_realm();
    }
    /// Ends the current generation: its open entry is sealed, its contributions
    /// go, and nothing later reaches or comes from it.
    fn retire_realm(&mut self) {
        let live = self.realm.take();
        if let Some(live) = &live {
            live.realm.close();
            live.host.seal();
        }
        drop(live);
        *self.transport.lock().unwrap() = None;
        self.router = None;
        self.asking = None;
        self.asking_culprit = None;
        self.dom_waiting.clear();
        self.deferred.clear();
        self.canvases.clear();
        self.shared.reader.store(0, Ordering::Release);
        self.shared.update(|p| {
            p.view = 0;
            p.batches.clear();
            p.canvases.clear();
        });
        drop(self.config.kernel.state_transaction(StateTransaction {
            expected_revision: None,
            ops: vec![StateOp::DisposeOwner { owner: self.writer }],
        }));
        self.config.kernel.retire_realm(self.writer);
        self.retired = true;
    }
    /// The task's execution id, when it was queued. When the realm's queue
    /// is full (a long or hung task) the source waits here, in order, and the
    /// owner keeps handling input (Stop, Quit) instead of waiting for room.
    fn eval(&mut self, source: &str) -> Option<u64> {
        let live = self.realm.as_ref()?;
        if !self.deferred.is_empty() {
            self.defer(source.to_owned());
            return None;
        }
        match live.realm.eval(source) {
            Ok(ticket) => Some(ticket.execution),
            Err(Error::Busy) => {
                self.defer(source.to_owned());
                None
            }
            Err(_) => {
                let _ = self.inbox.send(Input::Failed(self.writer.generation));
                None
            }
        }
    }
    fn defer(&mut self, source: String) {
        // A realm this far behind is hung; the watchdog asks about it, and a
        // stopped realm restarts from the kernel's state, not from this queue.
        if self.deferred.len() >= MAX_DEFERRED {
            self.deferred.pop_front();
        }
        self.deferred.push_back(source);
    }
    /// Queues what waited for room, while the realm takes it.
    fn retry_deferred(&mut self) {
        while let Some(source) = self.deferred.front() {
            let Some(live) = self.realm.as_ref() else {
                self.deferred.clear();
                return;
            };
            match live.realm.eval(source.as_str()) {
                Err(Error::Busy) => return,
                Ok(_) => {
                    self.deferred.pop_front();
                }
                Err(_) => {
                    self.deferred.clear();
                    let _ = self.inbox.send(Input::Failed(self.writer.generation));
                    return;
                }
            }
        }
    }
    fn handle(&mut self, message: Input) {
        match message {
            Input::Run(id, args) => {
                if !self.run_store(&id, &args) {
                    let source = format!("__compat.run({},{args})", json!(id));
                    let owner = self.command_owner(&id);
                    self.running = self.eval(&source).zip(owner);
                }
            }
            Input::StorePage(id, view, message) => {
                let live = self.stores.iter_mut().find(|s| s.extension.id == id);
                if let Some(store) = live.filter(|s| u64::from(s.owner.realm) == view) {
                    store.extension.page(message);
                }
                self.drain_store_dom(&id);
            }
            Input::StoreDom(id) => self.drain_store_dom(&id),
            Input::Panel(id, open) => {
                if open {
                    self.open.entry(id.clone()).or_default();
                } else {
                    self.open.remove(&id);
                }
                self.show_panel(&id, open);
            }
            Input::Viewport(width, height, scale) => {
                self.viewport = Some((width, height, scale));
                self.eval(&format!("__compat.viewport({width},{height},{scale})"));
            }
            Input::Place(id, rect) => {
                if let Some(placed) = self.open.get_mut(&id) {
                    *placed = rect;
                }
                self.place_panel(&id, rect);
            }
            Input::Answer(token, value) => {
                self.eval(&format!("__compat.answer({token},{value})"));
            }
            Input::Shell(state) => {
                if let Value::Object(facts) = &state {
                    self.shell.extend(facts.clone());
                }
                self.eval(&format!("__compat.shellState({state})"));
            }
            Input::Presented => {
                let frame = self.presented.lock().unwrap().take();
                if let Some(frame) = frame {
                    self.eval(&format!("__compat.presented({frame})"));
                }
            }
            Input::Rasters(batch, wire) => {
                let ask = match &wire["ask"] {
                    Value::Object(_) => wire["ask"].clone(),
                    _ => json!({}),
                };
                self.eval(&format!(
                    "__compat.answerRasters({batch},{},{ask})",
                    wire["needs"]
                ));
            }
            Input::Export(job, frozen, cancel, (tx, rx)) => {
                let start = export::Start {
                    job,
                    frozen,
                    cancel,
                    records: self.trusted_records(),
                    storage: crate::adapter::storage_of(&self.config.kernel),
                    frames: self.config.frames.clone(),
                    viewport: self.viewport,
                    text: self.config.text.as_ref().map(|text| text()),
                    rasters: self.config.rasters.clone(),
                    kernel: self.config.kernel.clone(),
                    shared: self.shared.clone(),
                    closing: self.closing.clone(),
                };
                let spawned = thread::Builder::new()
                    .name("pm-compat-export".into())
                    .spawn(move || export::run(start, tx, rx));
                if spawned.is_err() {
                    self.shared.update(|p| {
                        p.toasts
                            .push("The export's extensions could not start".into())
                    });
                }
            }
            Input::Menu(location, context, key) => {
                self.eval(&format!(
                    "__compat.gatherMenu({},{context},{key})",
                    json!(location)
                ));
            }
            Input::Moved => {
                let moved = self.moved.swap(0, Ordering::AcqRel);
                if let Some(live) = &self.realm {
                    let now = live.host.transport().time();
                    self.eval(&format!(
                        "__compat.moved({now},{},{})",
                        moved & TIME_MOVED != 0,
                        moved & PLAY_MOVED != 0
                    ));
                }
            }
            Input::Realm(writer, envelope) if writer == self.writer => self.realm_message(envelope),
            Input::Page(view, message) if view == u64::from(self.writer.generation) => {
                self.page(message)
            }
            Input::Dom => self.drain_dom(),
            Input::Canvases => self.collect_canvases(),
            Input::Health(id, health) => self.set_health(&id, health),
            Input::Enabled(id, enabled) => self.set_enabled(&id, enabled),
            Input::Booted(generation) if generation == self.writer.generation => {
                self.shared.update(|p| p.ready = true);
                self.reopen_panels(None);
            }
            Input::Rescan => self.rescan(),
            Input::FontsLoaded(keys) => {
                self.eval(&format!("__compat.fontsLoaded({keys})"));
            }
            Input::Failed(generation) if generation == self.writer.generation => {
                let culprit = self.activating_now();
                self.restart(culprit)
            }
            Input::Unresponsive(generation, execution)
                if generation == self.writer.generation && self.asking != Some(execution) =>
            {
                self.ask_unresponsive(execution)
            }
            Input::StopTask(execution, stop) => {
                let culprit = if self.asking == Some(execution) {
                    self.asking = None;
                    self.asking_culprit.take()
                } else {
                    None
                };
                let stopped = stop && self.realm.as_ref().is_some_and(|l| l.realm.stop(execution));
                if stopped {
                    self.restart(culprit);
                }
            }
            Input::Realm(..)
            | Input::Page(..)
            | Input::Booted(_)
            | Input::Failed(_)
            | Input::Unresponsive(..)
            | Input::Kernel
            | Input::Close => (),
        }
    }
    fn tick_stores(&mut self) {
        let revision = self
            .config
            .kernel
            .published(&self.config.project)
            .map(|published| published.revision());
        if revision.is_none() || revision == self.store_revision {
            return;
        }
        self.store_revision = revision;
        for store in &self.stores {
            store.extension.tick(revision.unwrap_or_default());
        }
    }
    /// The last provider of `id` among this window's writers, and which helper it
    /// is if it is not the realm. Other windows' helpers hold their own entries.
    fn provider(&self, registry: &str, id: &str) -> Option<(Option<usize>, Registration)> {
        let state = self.config.kernel.state();
        let store = |owner: Writer| self.stores.iter().position(|s| s.owner == owner);
        let entry = state.registries.get(registry)?.iter().rev().find(|entry| {
            entry.id == id && (entry.owner == self.writer || store(entry.owner).is_some())
        })?;
        Some((store(entry.owner), entry.clone()))
    }
    fn store_of(&self, panel: &str) -> Option<&StoreExtension> {
        let (store, _) = self.provider("panels", panel)?;
        Some(&self.stores[store?].extension)
    }
    fn show_panel(&mut self, id: &str, open: bool) {
        if let Some(store) = self.store_of(id) {
            if open {
                store.open_panel(id);
            } else {
                store.close_panel(id);
            }
            return;
        }
        let verb = if open { "openPanel" } else { "closePanel" };
        self.eval(&format!("__compat.{verb}({})", json!(id)));
    }
    fn place_panel(&mut self, id: &str, rect: Option<[f32; 4]>) {
        if let Some(store) = self.store_of(id) {
            store.place_panel(id, rect);
            return;
        }
        let source = format!("__compat.placePanel({},{})", json!(id), json!(rect));
        self.eval(&source);
    }
    /// A rebuilt realm or helper starts with no panels open. `store` is the helper
    /// that just started; `None` is the realm.
    fn reopen_panels(&mut self, store: Option<Writer>) {
        for (id, rect) in self.open.clone() {
            let owner = self
                .provider("panels", &id)
                .and_then(|(index, _)| index.map(|index| self.stores[index].owner));
            if owner == store {
                self.show_panel(&id, true);
                if rect.is_some() {
                    self.place_panel(&id, rect);
                }
            }
        }
    }
    /// A full page queue leaves batches waiting. The helper's document holds what
    /// it could not send, and an admitted batch is never dropped.
    fn drain_store_dom(&mut self, id: &str) {
        let Some(store) = self.stores.iter_mut().find(|s| s.extension.id == id) else {
            return;
        };
        let mut queue = StorePageQueue(&self.shared, (id.to_owned(), u64::from(store.owner.realm)));
        let mut pending = store.views.pending.lock().unwrap();
        while let Some(batch) = pending.front() {
            if matches!(
                store.extension.enqueue(batch.clone(), &mut queue),
                Err(pm_compat_host::Error::Backpressure)
            ) {
                return;
            }
            pending.pop_front();
        }
        drop(pending);
        if store.views.refused.swap(false, Ordering::AcqRel) {
            store.extension.flush();
        }
    }
    /// A confined helper owns this command: its callback is a handle in there.
    fn run_store(&self, id: &str, args: &Value) -> bool {
        let Some((Some(store), entry)) = self.provider("commands", id) else {
            return false;
        };
        if let Some(handle) = entry.definition.get("run").and_then(|run| run.as_f64()) {
            self.stores[store].extension.run(handle as u64, args);
        }
        true
    }
    /// Publishes what a helper registered through the same path the realm uses.
    fn publish_store(&self) {
        let state = self.config.kernel.state();
        let mut store = json!({});
        let mut definitions = vec![];
        for store_entry in &self.stores {
            let (owner, extension) = (&store_entry.owner, &store_entry.extension);
            let mut own = json!({"effects":[],"transitions":[],"layers":[]});
            for (kind, registry) in [
                ("commands", "commands"),
                ("panels", "panels"),
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
                        serde_json::from_str::<Value>(&pm_model::json::to_canonical_string(
                            &entry.definition,
                        ))
                        .ok()
                    })
                    .map(|mut item| {
                        // Panels name the helper whose page draws them, and which run of it.
                        if kind == "panels" {
                            item["store"] = json!(extension.id);
                            item["view"] = json!(owner.realm);
                        }
                        item
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
        let Some(router) = &mut self.router else {
            return;
        };
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
            // A synchronous layout read is waiting for this answer: it goes to
            // the blocked realm directly (and through its queue as usual).
            if message["kind"] == "layout"
                && let (Some(request), Some(live)) = (message["request"].as_u64(), &self.realm)
            {
                live.events
                    .measuring
                    .lock()
                    .unwrap()
                    .answer(request, &message);
            }
            self.eval(&format!(
                "__compat.dom.receive({message});__compat.dom.flush()"
            ));
        }
        self.drain_dom();
    }
    /// Batches the page queue cannot take stay here and in the realm, whose
    /// document coalesces later changes: nothing is dropped and nothing grows.
    fn drain_dom(&mut self) {
        let (Some(live), Some(router)) = (&self.realm, &mut self.router) else {
            return;
        };
        live.events.dom_pending.store(false, Ordering::Release);
        let mut queue = PageQueue(&self.shared);
        let mut admitted = false;
        let mut full = false;
        loop {
            if self.dom_waiting.is_empty() {
                self.dom_waiting.extend(live.host.take_dom_batches());
            }
            let Some(batch) = self.dom_waiting.front() else {
                break;
            };
            // A rejected tree is an extension fault; the page keeps its last state.
            if matches!(
                router.enqueue(batch.clone(), &mut queue),
                Err(pm_compat_host::Error::Backpressure)
            ) {
                full = true;
                break;
            }
            self.dom_waiting.pop_front();
            admitted = true;
        }
        if admitted {
            (self.shared.wake)();
        }
        // Also the shim's next flush: it holds a batch the realm's queue refused.
        if !full {
            self.eval("__compat.flushCanvases?.()");
        }
    }
    /// Only a page that is reading gets pixels, and only surfaces that changed.
    fn collect_canvases(&mut self) {
        let Some(live) = &self.realm else { return };
        live.events.canvas_pending.store(false, Ordering::Release);
        let view = u64::from(self.writer.generation);
        if self.shared.reader.load(Ordering::Acquire) != view {
            return;
        }
        let frames = live
            .host
            .changed_canvas_frames(&self.canvases)
            .unwrap_or_default();
        if frames.is_empty() {
            return;
        }
        for frame in &frames {
            self.canvases
                .insert((frame.node, frame.surface.clone()), frame.version);
        }
        self.shared.update(|p| {
            for frame in frames {
                p.canvases
                    .insert((frame.node, frame.surface.clone()), frame);
            }
        });
    }
    fn set_health(&mut self, id: &str, health: Value) {
        for record in &mut self.records {
            if record["id"] == id {
                record["health"] = health.clone();
            }
        }
        self.shared.update(|p| {
            if let Some(record) = p.records.get_mut(id) {
                record["health"] = health;
            }
        });
        self.publish_entries();
    }
    /// The saved preference; turning an extension on also ends a D11 stop.
    fn persist_enabled(&self, id: &str, enabled: bool) {
        let _ = update_state::<String>(&self.config.kernel, |state| {
            Ok(state
                .extensions
                .get(id)
                .filter(|entry| entry.enabled != enabled || (enabled && stopped_this_run(entry)))
                .map(|entry| {
                    let mut entry = pm_kernel::state::ExtensionEntry {
                        enabled,
                        ..entry.clone()
                    };
                    if enabled {
                        clear_stop(&mut entry);
                    }
                    vec![StateOp::Extension(entry)]
                }))
        });
    }
    fn set_enabled(&mut self, id: &str, enabled: bool) {
        self.persist_enabled(id, enabled);
        let mut confined = false;
        for record in &mut self.records {
            if record["id"] == id {
                record["enabled"] = json!(enabled);
                confined = record["confined"] == true;
                // What stopped it may be gone; what it is made of has not changed.
                let stopped = ["disabled", "runtime-error", "activation-error"]
                    .contains(&record["health"]["state"].as_str().unwrap_or_default());
                if enabled && stopped {
                    record["health"] = json!({"state":"ok"});
                }
            }
        }
        if confined {
            self.sync_stores();
            self.publish_store();
        } else {
            let records = self.realm_plan();
            self.eval(&format!(
                "__compat.update({records},{},\"enable\")",
                json!([id])
            ));
        }
        self.publish_records();
    }
    fn enabled(&self, id: &str) -> bool {
        let state = self.config.kernel.state();
        state
            .extensions
            .get(id)
            .is_none_or(|entry| entry.enabled && !stopped_this_run(entry))
    }
    /// D11 Stop: off for this run (every window), not in the saved
    /// preference; Settings shows it off and turning it on clears this.
    fn stop_for_this_run(&self, id: &str) {
        let _ = update_state::<String>(&self.config.kernel, |state| {
            Ok(state
                .extensions
                .get(id)
                .filter(|e| !stopped_this_run(e))
                .map(|entry| {
                    let mut entry = entry.clone();
                    if let Some(source) = entry.source.as_object_mut() {
                        let mut record = source
                            .get("hostRecord")
                            .and_then(pm_model::Value::as_object)
                            .cloned()
                            .unwrap_or_default();
                        record.insert("stopped", pm_model::Value::Bool(true));
                        source.insert("hostRecord", pm_model::Value::Object(record));
                    }
                    vec![StateOp::Extension(entry)]
                }))
        });
    }
    /// Settings and MCP read extensions from kernel state, as they do built-ins.
    fn publish_entries(&self) {
        let _ = update_state::<String>(&self.config.kernel, |state| {
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
                    // Stopped this run: kept while the app runs, gone at the
                    // next launch's first publication.
                    let stopped = !FIRST_ENTRIES.load(Ordering::Acquire)
                        && previous.is_some_and(stopped_this_run);
                    let mut summary = json!({"name":record["manifest"]["name"],"about":record["manifest"]["description"],
                    "health":record["health"],"trust":record["trust"],"contributes":record["manifest"]["contributes"]});
                    if stopped {
                        summary["stopped"] = json!(true);
                    }
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
            Ok((!ops.is_empty()).then_some(ops))
        });
        FIRST_ENTRIES.store(false, Ordering::Release);
    }
    /// A Settings toggle or an agent's edit lands in state or on disk; follow it.
    fn follow_state(&mut self) {
        let state = self.config.kernel.state();
        let toggled: Vec<(String, bool)> = self
            .records
            .iter()
            .filter_map(|record| {
                let id = record["id"].as_str()?;
                let entry = state.extensions.get(id)?;
                let enabled = entry.enabled && !stopped_this_run(entry);
                (record["enabled"] != enabled).then(|| (id.to_owned(), enabled))
            })
            .collect();
        for (id, enabled) in toggled {
            self.set_enabled(&id, enabled);
        }
    }
    fn stamp(&self) -> u64 {
        discovery::stamp(&self.config.roots)
    }
    /// The record alone. Whether a Store extension's helper runs is `sync_stores`.
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
        let mut record = discovery::record(
            found,
            enabled,
            graph.as_ref().map(|g| g.to_json()).map_err(Clone::clone),
        );
        let surface = record["manifest"]["apiVersion"].as_u64() >= Some(4);
        if record["trust"] != "store" {
            // Surface extensions get their own realm from the panel host.
            if surface {
                record["surface"] = json!(true);
            }
            return record;
        }
        // Someone else's code: confined whatever its state, and never in the plan
        // of the trusted realm or the panel host, so no later toggle can move it.
        record["confined"] = json!(true);
        if record["health"]["state"] != "ok" {
            return record;
        }
        let wants_full_access = record["manifest"]["permissions"]
            .as_array()
            .is_some_and(|p| p.iter().any(|p| p == "full-access"));
        if surface {
            // The panel host has no permission model, so it takes no Store code.
            record["health"] = json!({"state":"build-error",
                "error":"Store extensions run on API versions 1 to 3"});
        } else if wants_full_access {
            record["health"] = json!({"state":"needs-trust"});
        } else if let Ok(graph) = graph {
            self.confined
                .insert(found.id.clone(), (found.clone(), graph));
        }
        record
    }
    /// Runs exactly the helpers the records want: stops those disabled, removed
    /// or changed, starts what is missing, and leaves every other one alone.
    fn sync_stores(&mut self) {
        let wanted: BTreeMap<String, String> = self
            .records
            .iter()
            .filter(|r| {
                r["confined"] == true && r["enabled"] == true && r["health"]["state"] == "ok"
            })
            .filter_map(|r| {
                Some((
                    r["id"].as_str()?.to_owned(),
                    r["graph"]["hash"].as_str()?.to_owned(),
                ))
            })
            .filter(|(id, _)| self.confined.contains_key(id))
            .collect();
        let (keep, stop): (Vec<_>, Vec<_>) = std::mem::take(&mut self.stores)
            .into_iter()
            .partition(|store| wanted.get(&store.extension.id) == Some(&store.hash));
        self.stores = keep;
        for store in stop {
            self.stop_store(store);
        }
        for (id, hash) in wanted {
            if !self.stores.iter().any(|store| store.extension.id == id) {
                self.start_store(&id, hash);
            }
        }
    }
    fn start_store(&mut self, id: &str, hash: String) {
        let Some((found, graph)) = self.confined.get(id).cloned() else {
            return;
        };
        let owner = Writer {
            realm: NEXT_REALM.fetch_add(1, Ordering::Relaxed),
            generation: GENERATION,
        };
        let views = Arc::new(ViewEvents {
            id: id.to_owned(),
            shared: self.shared.clone(),
            inbox: self.inbox.clone(),
            pending: Mutex::default(),
            refused: AtomicBool::new(false),
        });
        let started = StoreExtension::start(
            Arc::new(self.config.kernel.clone()),
            &self.config.project,
            "native",
            &found,
            &graph,
            owner,
            views.clone(),
        );
        match started {
            Ok(extension) => {
                self.shared
                    .sessions
                    .lock()
                    .unwrap()
                    .insert(extension.id.clone(), extension.session.clone());
                self.stores.push(StoreLive {
                    owner,
                    hash,
                    extension,
                    views,
                });
                self.reopen_panels(Some(owner));
            }
            Err(error) => {
                for record in &mut self.records {
                    if record["id"] == id {
                        record["health"] =
                            json!({"state":"activation-error","error":error.to_string()});
                    }
                }
            }
        }
    }
    /// Disposing the helper revokes its grants; the registrations go with its writer.
    fn stop_store(&mut self, store: StoreLive) {
        let (id, owner) = (store.extension.id.clone(), store.owner);
        drop(store);
        self.shared.sessions.lock().unwrap().remove(&id);
        drop(self.config.kernel.state_transaction(StateTransaction {
            expected_revision: None,
            ops: vec![StateOp::DisposeOwner { owner }],
        }));
        self.shared.update(|p| {
            p.store_batches.retain(|(store, _), _| *store != id);
        });
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
            .filter(|r| {
                r["surface"] == true && r["health"]["state"] == "ok" && r["enabled"] == true
            })
            .cloned()
            .collect();
        self.shared.update(|p| {
            p.records = records;
            p.surfaces = surfaces;
        });
        self.publish_entries();
    }
    fn start_realm(&mut self, clock: Option<(f64, bool)>) -> Result<()> {
        if std::mem::take(&mut self.retired) {
            self.writer.generation += 1;
            forward_mailbox(&self.config.kernel, self.writer, self.inbox.clone());
        }
        let events = Arc::new(Events {
            shared: self.shared.clone(),
            kernel: self.config.kernel.clone(),
            writer: self.writer,
            inbox: self.inbox.clone(),
            activating: Mutex::default(),
            measuring: Mutex::default(),
            page_missed: AtomicBool::new(false),
            dom_pending: AtomicBool::new(false),
            canvas_pending: AtomicBool::new(false),
        });
        let adapter = KernelAdapter::new(
            self.config.kernel.clone(),
            &self.config.project,
            crate::PROJECT,
            self.writer,
        );
        let transport = Transport::new(self.config.owner.clone());
        if let Some((time, playing)) = clock {
            transport.publish(time, playing);
        }
        let host = Arc::new(NativeHost::new(
            adapter,
            transport,
            Box::new(events.clone()),
        ));
        if let Some(text) = &self.config.text {
            host.set_text(text());
        }
        host.set_frames(self.config.frames.clone());
        host.set_rasters(self.config.rasters.clone());
        let pool = crate::pool(&format!("window:{}", self.config.project), host.clone())?;
        let realm = pool.realm(RealmKind::Compat)?;
        crate::boot_realm(&realm, &host, "")?;
        *self.transport.lock().unwrap() = Some(host.clone());
        self.router = Some(
            RemoteDomPanel::new(
                PANEL,
                Session {
                    panel: "compat".into(),
                    generation: PAGE_SESSION,
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
        let view = u64::from(self.writer.generation);
        self.shared.update(|p| p.view = view);
        Ok(())
    }
    fn boot(&mut self) {
        let found = discovery::scan(&self.config.roots);
        if found.is_empty() {
            // Nothing to run: no realm, no memory, no bundle evaluation.
            self.shared.update(|p| p.ready = true);
            return;
        }
        if let Err(error) = self.start_realm(None) {
            self.shared.update(|p| {
                p.toasts
                    .push(format!("Extensions could not start: {error}"))
            });
            return;
        }
        let live = self.realm.as_ref().unwrap();
        let compiler = match SvelteCompiler::new(&live.pool, 256 * 1024 * 1024, 8 * 1024 * 1024) {
            Ok(compiler) => compiler,
            Err(error) => {
                self.shared.update(|p| {
                    p.toasts
                        .push(format!("Extensions could not compile: {error}"))
                });
                return;
            }
        };
        self.confined.clear();
        self.records = found.iter().map(|f| self.compile(&compiler, f)).collect();
        compiler.realm().close();
        self.sync_stores();
        self.publish_records();
        self.publish_store();
        let records = self.realm_plan();
        self.replay_window();
        self.eval(&format!("__compat.boot({records})"));
    }
    /// A new realm learns the window it lives in before anything activates.
    fn replay_window(&mut self) {
        if let Some((width, height, scale)) = self.viewport {
            self.eval(&format!("__compat.viewport({width},{height},{scale})"));
        }
        if !self.shell.is_empty() {
            let shell = Value::Object(self.shell.clone());
            self.eval(&format!("__compat.shellState({shell})"));
        }
    }
    /// The trusted records the window realm is about to run: its wrappers
    /// report their code identity from them (the disk render cache keys
    /// override frames by it; export realms pin these same records).
    fn realm_plan(&self) -> Value {
        let records = self.trusted_records();
        if let Some(live) = &self.realm {
            live.host.set_code(&records);
        }
        records
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
        self.confined.clear();
        let records: Vec<_> = found.iter().map(|f| self.compile(&compiler, f)).collect();
        compiler.realm().close();
        let changed: Vec<_> = records
            .iter()
            .filter(|next| {
                !self.records.iter().any(|old| {
                    old["id"] == next["id"] && old["graph"]["hash"] == next["graph"]["hash"]
                })
            })
            .map(|record| record["id"].clone())
            .collect();
        self.records = records;
        self.sync_stores();
        self.publish_records();
        self.publish_store();
        let records = self.realm_plan();
        self.eval(&format!(
            "__compat.update({records},{},\"rescan\")",
            Value::Array(changed)
        ));
    }
    /// D11: "not responding — Wait / Stop", named for the extension that was
    /// activating when there is one. The task keeps running while the user decides.
    /// The extension is the one whose code the watchdog found on the stack
    /// (any task: a raster override, an event, a timer, a command), else the
    /// one activating or whose command runs. Stop turns it off, so the
    /// rebuilt realm doesn't run into the same hang again.
    fn ask_unresponsive(&mut self, execution: u64) {
        let Some(live) = &self.realm else { return };
        let activating = live.events.activating.lock().unwrap().clone();
        let command = self
            .running
            .as_ref()
            .filter(|(task, _)| *task == execution)
            .map(|(_, owner)| owner.clone());
        let culprit = live
            .realm
            .culprit(execution)
            .or(activating)
            .or(command)
            .filter(|id| self.records.iter().any(|r| r["id"] == id.as_str()));
        let name = culprit.as_ref().map(|id| {
            self.records
                .iter()
                .find(|r| r["id"] == id.as_str())
                .and_then(|r| r["manifest"]["name"].as_str())
                .unwrap_or(id)
                .to_owned()
        });
        let disables = culprit.is_some();
        self.asking = Some(execution);
        self.asking_culprit = culprit;
        let request =
            json!({"kind":"unresponsive","execution":execution,"name":name,"disables":disables});
        self.shared.update(|p| p.requests.push(request));
    }
    fn activating_now(&self) -> Option<String> {
        let live = self.realm.as_ref()?;
        live.events.activating.lock().unwrap().clone()
    }
    fn command_owner(&self, id: &str) -> Option<String> {
        let contributions = self
            .shared
            .published
            .lock()
            .unwrap()
            .contributions
            .clone()?;
        contributions["commands"]
            .as_array()?
            .iter()
            .find(|c| c["id"] == id)?["owner"]
            .as_str()
            .map(str::to_owned)
    }
    /// A hung or failed realm is rebuilt without the extension that broke it. That
    /// extension stays off until someone turns it back on, here and in every other
    /// window, and the next launch does not run into it again.
    fn restart(&mut self, culprit: Option<String>) {
        let Some(live) = &self.realm else { return };
        live.events.activating.lock().unwrap().take();
        let transport = live.host.transport();
        let clock = (transport.time(), transport.playing());
        self.retire_realm();
        // Repeated failures nobody can name stop the realm; a Stop that turns
        // its culprit off is not a loop.
        if culprit.is_none() {
            self.restarts += 1;
        }
        if let Some(id) = &culprit {
            self.stop_for_this_run(id);
            let mut name = id.clone();
            for record in &mut self.records {
                if record["id"] == id.as_str() {
                    record["enabled"] = json!(false);
                    record["health"] =
                        json!({"state":"runtime-error","error":"Stopped responding"});
                    if let Some(n) = record["manifest"]["name"].as_str() {
                        name = n.to_owned();
                    }
                }
            }
            self.shared.update(|p| {
                p.toasts.push(format!(
                    "“{name}” is off until Powermove restarts. Turn it on in Settings ▸ Extensions."
                ))
            });
        }
        if self.restarts > MAX_RESTARTS {
            // Helpers stay; what the realm contributed went with it.
            let mut value = json!({});
            merge_store(&mut value, &self.shared.store.lock().unwrap());
            self.shared.update(|p| {
                p.contributions = Some(Arc::new(value));
                p.toasts.push(
                    "Extensions stopped after repeated failures. Reopen the project to retry."
                        .into(),
                );
            });
            self.publish_records();
            return;
        }
        match self.start_realm(Some(clock)) {
            Ok(()) => {
                self.publish_records();
                let records = self.realm_plan();
                self.replay_window();
                self.eval(&format!("__compat.boot({records})"));
            }
            Err(error) => self.shared.update(|p| {
                p.toasts
                    .push(format!("Extensions could not start: {error}"))
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_kernel::{HistoryCall, KernelActor, KernelConfig, OpenSource, Reply, Request};
    use std::{fs, path::Path, time::Instant};
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
            Self::start_with(root, crate::initial()["document"].clone(), None)
        }
        fn start_with(
            root: tempfile::TempDir,
            document: Value,
            rasters: Option<crate::RasterHooks>,
        ) -> Self {
            Self::start_full(root, document, rasters, None)
        }
        fn start_full(
            root: tempfile::TempDir,
            document: Value,
            rasters: Option<crate::RasterHooks>,
            frames: Option<crate::FrameRenderer>,
        ) -> Self {
            let actor = KernelActor::spawn(KernelConfig::default());
            let kernel = actor.handle();
            let document = pm_model::json::parse(&document.to_string()).unwrap();
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
                frames,
                rasters,
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
        /// Waits for the watchdog's question and returns the task it names.
        fn answer_unresponsive(&self, name: Option<&str>) -> u64 {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let asked = self
                    .host
                    .take_requests()
                    .into_iter()
                    .find(|r| r["kind"] == "unresponsive");
                if let Some(request) = asked {
                    assert_eq!(request["name"].as_str(), name);
                    return request["execution"].as_u64().unwrap();
                }
                assert!(Instant::now() < deadline, "no unresponsive prompt");
                thread::sleep(Duration::from_millis(20));
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
    fn appkit_answers_reach_realm_closures_and_storage_outlives_the_realm() {
        let root = tempfile::tempdir().unwrap();
        let extensions = root.path().join("extensions");
        extension(
            &extensions,
            "synthetic-ui",
            &[(
                "index.ts",
                "export default function(api){
                   const rename=(name)=>api.history.do('Rename',()=>{api.project.get().name=name});
                   const round=(api.storage.get('round')||0)+1;
                   api.storage.set('round',round);
                   api.panels.register({id:'synthetic-ui.panel',title:'Synthetic',build(){}});
                   api.menus.contribute('panel:context',(ctx)=>[{label:'For '+ctx.panelId,run(){rename('tab')}}]);
                   api.commands.register({id:'synthetic-ui.round',label:'Round',run(){rename('round '+round)}});
                   api.commands.register({id:'synthetic-ui.open',label:'Open',run(){api.panels.open('synthetic-ui.panel')}});
                   api.commands.register({id:'synthetic-ui.icon',label:'Icon',run(){rename(api.ui.icon('plus').slice(0,4)+api.extensions.list().length)}});
                   api.commands.register({id:'synthetic-ui.off',label:'Off',run(){api.extensions.setEnabled('synthetic-ui',false)}});
                   api.commands.register({id:'synthetic-ui.confirm',label:'Confirm',run(){
                     api.ui.confirm('Sure?','Body').then((yes)=>rename(yes?'confirmed':'declined'))}});
                   api.commands.register({id:'synthetic-ui.menu',label:'Menu',run(){
                     api.ui.toast('Choose');
                     api.ui.menu({x:4,y:5},[{header:'Pick'},'-',{label:'Rename',run(){rename('menu')}}])}});
                 }",
            )],
        );
        let fixture = Fixture::start(root);
        fixture.until("activation", |host| {
            host.is_ready() && host.contributions().is_some()
        });
        let request = |what: &str| {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                if let Some(request) = fixture.host.take_requests().pop() {
                    return request;
                }
                assert!(Instant::now() < deadline, "{what}");
                thread::sleep(Duration::from_millis(5));
            }
        };
        let named = |name: &str| {
            fixture.until(name, |_| fixture.name() == name);
        };
        fixture.host.run_command("synthetic-ui.open", json!([]));
        let reveal = request("panel reveal");
        assert_eq!(
            (&reveal["kind"], &reveal["id"], &reveal["open"]),
            (&json!("panel"), &json!("synthetic-ui.panel"), &json!(true))
        );
        fixture.host.run_command("synthetic-ui.confirm", json!([]));
        let sheet = request("confirm");
        assert_eq!(
            (&sheet["title"], &sheet["body"]),
            (&json!("Sure?"), &json!("Body"))
        );
        fixture
            .host
            .answer(sheet["token"].as_u64().unwrap(), json!(true));
        named("confirmed");
        fixture.host.run_command("synthetic-ui.menu", json!([]));
        let menu = request("menu");
        assert_eq!(
            (menu["x"].as_f64(), menu["y"].as_f64()),
            (Some(4.), Some(5.))
        );
        assert_eq!(menu["items"].as_array().unwrap().len(), 3);
        assert_eq!(fixture.host.take_toasts(), ["Choose"]);
        fixture.host.answer(
            menu["token"].as_u64().unwrap(),
            menu["items"][2]["run"].clone(),
        );
        named("menu");
        fixture
            .host
            .gather_menu("panel:context", json!({"panelId":"first"}), 7);
        let gathered = request("gathered");
        assert_eq!(gathered["key"], 7);
        assert_eq!(gathered["items"][0]["label"], "For first");
        fixture.host.answer(
            gathered["token"].as_u64().unwrap(),
            gathered["items"][0]["run"].clone(),
        );
        named("tab");
        // Storage lives in durable `ext.*` settings, so a later window reads it.
        fixture.until("storage", |_| {
            fixture
                .kernel
                .state()
                .settings
                .contains_key("ext.synthetic-ui")
        });
        fixture.host.run_command("synthetic-ui.round", json!([]));
        named("round 1");
        fixture.host.run_command("synthetic-ui.icon", json!([]));
        named("<svg1");
        let mut later = ExtensionHost::start(HostConfig {
            kernel: fixture.kernel.clone(),
            project: "window".into(),
            roots: vec![extensions],
            cache: fixture._root.path().join("cache-later"),
            owner: None,
            text: None,
            wake: Arc::new(|| ()),
            frames: None,
            rasters: None,
        });
        let deadline = Instant::now() + Duration::from_secs(20);
        while !later.is_ready() || later.contributions().is_none() {
            assert!(Instant::now() < deadline, "second host");
            thread::sleep(Duration::from_millis(5));
        }
        later.run_command("synthetic-ui.round", json!([]));
        named("round 2");
        later.run_command("synthetic-ui.off", json!([]));
        fixture.until("disabled", |_| {
            !fixture.kernel.state().extensions["synthetic-ui"].enabled
        });
        later.close();
        let _ = later.worker.take().map(thread::JoinHandle::join);
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
        // The name publishes before the extension's history entry closes.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let ack = fixture
                .kernel
                .call_blocking(
                    "window",
                    Request::new(pm_kernel::Caller::native(), HistoryCall::Undo),
                )
                .unwrap();
            if matches!(ack.reply, Reply::Bool(true)) {
                break;
            }
            assert!(Instant::now() < deadline, "undo stayed refused");
            thread::sleep(Duration::from_millis(5));
        }
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
        assert_eq!(
            provider.definition.get("label").unwrap().as_str(),
            Some("Tint")
        );
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
                       api.events.on('project:changed',()=>{api.storage.set('seen',true)});
                     }",
                ),
                (".powermove-store.json", "{\"repoId\":\"r\",\"releaseId\":\"synthetic\"}"),
            ],
        );
        extension(
            &root.path().join("extensions"),
            "synthetic-editor",
            &[(
                "index.ts",
                "export default function(api){api.commands.register({id:'synthetic-editor.rename',label:'Rename',
                   run(){api.history.do('Rename',()=>{api.project.get().name='Moved'})}})}",
            )],
        );
        let fixture = Fixture::start(root);
        fixture.until("activation", |host| {
            host.is_ready() && host.contributions().is_some()
        });
        assert_eq!(
            fixture.health("synthetic-store"),
            "ok",
            "{:?}",
            fixture.host.records()
        );
        let state = fixture.kernel.state();
        assert!(state.provider("effects", "synthetic-store.tint").is_some());
        assert!(state.provider("commands", "synthetic-store.mark").is_some());
        let contributions = fixture.host.contributions().unwrap();
        let definitions = contributions["definitions"].as_array().unwrap();
        assert_eq!(definitions[0]["origin"], "store");
        assert_eq!(definitions[0]["id"], "synthetic-store.tint");
        assert!(command(&fixture, "synthetic-store.mark"));
        // A listener in the helper hears a project change and writes through its
        // own storage, proving the delivery end to end.
        fixture
            .host
            .run_command("synthetic-editor.rename", json!([]));
        fixture.until("listener", |_| {
            fixture
                .kernel
                .state()
                .extensions
                .get("synthetic-store")
                .and_then(|entry| entry.source.get("storePrivateStorageV1").cloned())
                .is_some_and(|storage| {
                    pm_model::json::to_canonical_string(&storage).contains("seen")
                })
        });
        // A Settings toggle stops the helper and starts it again; the trusted
        // realm never runs Store code, whatever the toggle order.
        let toggle = |enabled: bool| {
            let entry = fixture.kernel.state().extensions["synthetic-store"].clone();
            fixture
                .kernel
                .state_transaction(StateTransaction {
                    expected_revision: None,
                    ops: vec![StateOp::Extension(pm_kernel::state::ExtensionEntry {
                        enabled,
                        ..entry
                    })],
                })
                .wait()
                .unwrap()
                .unwrap();
        };
        toggle(false);
        // The kernel's registry follows the published list through its own queue.
        fixture.until("helper stopped", |host| {
            host.store_session("synthetic-store").is_none()
                && !command(&fixture, "synthetic-store.mark")
                && fixture
                    .kernel
                    .state()
                    .provider("commands", "synthetic-store.mark")
                    .is_none()
        });
        toggle(true);
        fixture.until("helper again", |host| {
            host.store_session("synthetic-store").is_some()
                && command(&fixture, "synthetic-store.mark")
        });
        let state = fixture.kernel.state();
        let owner = state.provider("commands", "synthetic-store.mark").unwrap();
        assert!(
            owner.token.starts_with("store:synthetic-store:"),
            "{owner:?}"
        );
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
        // A Store install never reaches the panel host, which has no permissions.
        fs::create_dir_all(extensions.join("synthetic-v4-store")).unwrap();
        fs::write(
            extensions.join("synthetic-v4-store/manifest.json"),
            json!({"id":"synthetic-v4-store","name":"Synthetic","version":"1.0.0","apiVersion":4,"entry":"panel.tsx"}).to_string(),
        )
        .unwrap();
        fs::write(
            extensions.join("synthetic-v4-store/panel.tsx"),
            "export const view = () => <text>Store</text>;",
        )
        .unwrap();
        fs::write(
            extensions.join("synthetic-v4-store/.powermove-store.json"),
            "{\"repoId\":\"r\",\"releaseId\":\"synthetic\"}",
        )
        .unwrap();
        let fixture = Fixture::start(root);
        fixture.until("activation", |host| host.is_ready());
        assert_eq!(fixture.health("synthetic-v4-store"), "build-error");
        let surfaces = fixture.host.surfaces();
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0]["id"], "synthetic-v4");
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
            &[(
                "index.ts",
                "export default function(){throw new Error('synthetic')}",
            )],
        );
        extension(
            &extensions,
            "synthetic-broken",
            &[(
                "index.ts",
                "export default function(api){ this is not javascript",
            )],
        );
        extension(
            &extensions,
            "synthetic-missing",
            &[(
                "index.ts",
                "import './nowhere'; export default function(){}",
            )],
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
        // D11: the watchdog asks, naming the extension; nothing stops on its own.
        let asked = fixture.answer_unresponsive(Some("synthetic-hangs"));
        assert_eq!(fixture.host.view(), 1, "still the first realm while asking");
        fixture.host.answer_unresponsive(asked, true);
        fixture.until("recovery", |host| {
            let stopped = host
                .records()
                .iter()
                .any(|r| r["id"] == "synthetic-hangs" && r["health"]["state"] == "runtime-error");
            stopped
                && host.is_ready()
                && host.contributions().is_some_and(|c| {
                    c["commands"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|c| c["id"] == "synthetic-good.run")
                })
        });
        assert_eq!(fixture.health("synthetic-good"), "ok");
        // D11 Stop is for this run: the saved preference stays on, and the
        // stop keeps the culprit out of the rebuilt realm.
        let entry = |fixture: &Fixture| {
            fixture
                .kernel
                .state()
                .extensions
                .get("synthetic-hangs")
                .cloned()
                .unwrap()
        };
        assert!(entry(&fixture).enabled);
        assert!(stopped_this_run(&entry(&fixture)));
        let view = fixture.host.view();
        assert_eq!(view, 2, "one rebuild, one new page");
        thread::sleep(Duration::from_millis(1500));
        assert_eq!(fixture.host.view(), view, "no second hang");
        assert_eq!(fixture.health("synthetic-hangs"), "runtime-error");
        // Turning it on (Settings ▸ Extensions) runs it again: it hangs again,
        // and the watchdog asks about it again.
        let mut on = entry(&fixture);
        clear_stop(&mut on);
        drop(fixture.kernel.state_transaction(StateTransaction {
            expected_revision: None,
            ops: vec![StateOp::Extension(on)],
        }));
        let asked = fixture.answer_unresponsive(Some("synthetic-hangs"));
        fixture.host.answer_unresponsive(asked, true);
        fixture.until("stopped again", |host| host.view() == 3 && host.is_ready());
        assert!(entry(&fixture).enabled);
        let view = fixture.host.view();
        // A hung command with an open entry: the rebuild seals it, so native undo
        // is not left refused behind an entry no one will close.
        extension(
            &fixture._root.path().join("extensions"),
            "synthetic-stuck",
            &[(
                "index.ts",
                "export default function(api){api.commands.register({id:'synthetic-stuck.run',label:'Stuck',
                   run(){api.history.begin('Stuck');for(;;);}})}",
            )],
        );
        fixture.until("rescan", |host| command_of(host, "synthetic-stuck.run"));
        fixture.host.run_command("synthetic-stuck.run", json!([]));
        fixture.until("entry open", |_| {
            fixture.kernel.published("window").unwrap().entry.is_some()
        });
        // Wait keeps it running; the watchdog asks again later, and Stop ends it.
        let asked = fixture.answer_unresponsive(Some("synthetic-stuck"));
        fixture.host.answer_unresponsive(asked, false);
        assert_eq!(fixture.host.view(), view);
        let again = fixture.answer_unresponsive(Some("synthetic-stuck"));
        assert_eq!(again, asked, "the same task, asked again");
        fixture.host.answer_unresponsive(again, true);
        fixture.until("third rebuild", |host| host.view() == 4 && host.is_ready());
        let published = fixture.kernel.published("window").unwrap();
        assert!(published.entry.is_none(), "the open entry was sealed");
    }
    /// D11 outside activation and commands: the sheet names the extension whose
    /// code is running (read from the stack), and Stop turns that one off so
    /// the rebuilt realm doesn't run into the same hang again.
    #[test]
    fn a_hang_in_a_service_another_extension_calls_names_the_provider() {
        let root = tempfile::tempdir().unwrap();
        let extensions = root.path().join("extensions");
        extension(
            &extensions,
            "synthetic-provider",
            &[(
                "index.ts",
                "export default function(api){api.services.register('synthetic.slow',()=>{for(;;);})}",
            )],
        );
        extension(
            &extensions,
            "synthetic-caller",
            &[(
                "index.ts",
                "export default function(api){setTimeout(()=>api.services.get('synthetic.slow')?.(),300)}",
            )],
        );
        let fixture = Fixture::start(root);
        let asked = fixture.answer_unresponsive(Some("synthetic-provider"));
        fixture.host.answer_unresponsive(asked, true);
        fixture.until("rebuilt", |host| host.view() == 2 && host.is_ready());
        assert_eq!(fixture.health("synthetic-provider"), "runtime-error");
        assert_eq!(fixture.health("synthetic-caller"), "ok");
        thread::sleep(Duration::from_millis(1500));
        assert_eq!(fixture.host.view(), 2, "no second hang");
        assert!(
            !fixture
                .host
                .take_requests()
                .iter()
                .any(|r| r["kind"] == "unresponsive")
        );
    }
    /// A page that answers before the read starts waiting is not missed.
    type Answers = Arc<Mutex<Vec<(u64, Value)>>>;
    /// A window whose project has one text layer `t1`, the given extensions,
    /// and raster hooks that log every answer as `(batch, width | error)`.
    fn export_window(
        extensions: &[(&str, &str)],
        text: &str,
        frames: Option<crate::FrameRenderer>,
    ) -> (Fixture, Answers) {
        let root = tempfile::tempdir().unwrap();
        for (id, source) in extensions {
            extension(&root.path().join("extensions"), id, &[("index.ts", source)]);
        }
        let mut document = crate::initial()["document"].clone();
        document["layers"] = json!([{"id":"t1","type":"text","name":"T1","from":0,"dur":5,
            "d":{"text":text}}]);
        let answered: Answers = Arc::default();
        let log = answered.clone();
        let hooks = crate::RasterHooks {
            base: Arc::new(|_| Ok(None)),
            answers: Arc::new(move |batch, answers| {
                for answer in answers {
                    log.lock().unwrap().push((
                        batch,
                        match answer {
                            crate::RasterAnswer::Raster { width, .. } => json!(width),
                            crate::RasterAnswer::Failed(error) => json!(error),
                            other => json!(format!("{:?}", std::mem::discriminant(&other))),
                        },
                    ));
                }
            }),
            producing: Arc::new(|_, _, _| ()),
        };
        let fixture = Fixture::start_full(root, document, Some(hooks), frames);
        fixture.until("activation", |host| host.is_ready());
        (fixture, answered)
    }
    fn needs(fixture: &Fixture) -> Value {
        let comp = fixture
            .kernel
            .published("window")
            .unwrap()
            .snapshot
            .render_comp();
        json!([{"composition":comp,"layer":"t1","seconds":0,"scale":1}])
    }
    fn answer_of(answered: &Answers, batch: u64, wait: Duration) -> Value {
        let deadline = Instant::now() + wait;
        loop {
            if let Some((_, answer)) = answered.lock().unwrap().iter().find(|(b, _)| *b == batch) {
                return answer.clone();
            }
            assert!(Instant::now() < deadline, "batch {batch} was answered");
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn edit_text(fixture: &Fixture, command: &str, text: &str) {
        fixture.host.run_command(command, json!([text]));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let p = fixture.kernel.published("window").unwrap();
            if pm_model::json::to_canonical_string(&p.snapshot.document.to_value().unwrap())
                .contains(text)
            {
                return;
            }
            assert!(Instant::now() < deadline, "the edit reached the kernel");
            thread::sleep(Duration::from_millis(5));
        }
    }
    const SPY: &str = "export default function activate(api: any){
       api.commands.register({id:'spy.text',label:'Text',run(text: string){
         api.history.do('Text',()=>{api.project.get().layers[0].d.text=text})}});
       api.commands.register({id:'spy.store',label:'Store',run(v: string){ api.storage.set('extra', v) }});
       api.services.register('raster',(L: any)=>{
         if (L.id === 't1' && globalThis.capture && !globalThis.inside) {
           globalThis.inside = true;
           try { globalThis.capture(); } finally { globalThis.inside = false; }
         }
         const c=document.createElement('canvas');
         c.width=L.d.text.length + String(api.storage.get('extra') ?? '').length; c.height=1;
         api.project.get().name='written by '+L.d.text;
         return {cv:c,w:c.width,h:1,anchorX:0,anchorY:0}});}";
    /// The wrappers report the identity of the compiled code they run (the
    /// records an export realm pins): the same in another launch over the same
    /// files, different once the code changes. The disk render cache keys
    /// override frames by it.
    #[test]
    fn producing_reports_the_code_identity_of_the_records_the_realm_runs() {
        const RASTER: &str = "export default function(api: any){
           api.services.register('raster',(L: any)=>{
             const c=document.createElement('canvas'); c.width=1; c.height=1;
             return {cv:c,w:1,h:1,anchorX:0,anchorY:0}});}";
        type Calls = Arc<Mutex<Vec<(bool, u64)>>>;
        let launch = |source: &str| {
            let root = tempfile::tempdir().unwrap();
            extension(
                &root.path().join("extensions"),
                "wrap",
                &[("index.ts", source)],
            );
            let calls: Calls = Arc::default();
            let log = calls.clone();
            let hooks = crate::RasterHooks {
                base: Arc::new(|_| Ok(None)),
                answers: Arc::new(|_, _| ()),
                producing: Arc::new(move |on, _, code| log.lock().unwrap().push((on, code))),
            };
            let fixture =
                Fixture::start_with(root, crate::initial()["document"].clone(), Some(hooks));
            fixture.until("producing", |_| calls.lock().unwrap().iter().any(|c| c.0));
            let calls = calls.lock().unwrap().clone();
            assert!(calls.iter().all(|(_, code)| *code != 0), "{calls:?}");
            calls.iter().rev().find(|c| c.0).unwrap().1
        };
        let first = launch(RASTER);
        assert_eq!(launch(RASTER), first, "unchanged code: the same identity");
        assert_ne!(launch(&RASTER.replace("c.width=1", "c.width=2")), first);
    }
    #[test]
    fn an_export_realm_answers_from_its_frozen_project_code_and_storage_while_the_window_edits() {
        let (fixture, answered) = export_window(&[("spy", SPY)], "frozen", None);
        fixture.host.run_command("spy.store", json!(["ab"]));
        fixture.until("storage", |_| {
            fixture.kernel.state().settings.contains_key("ext.spy")
        });
        let frozen = fixture.kernel.published("window").unwrap();
        let needs = needs(&fixture);
        let inbox = fixture.host.raster_inbox();
        let cancel = Arc::new(AtomicBool::new(false));
        // The export starts: its realm pins the extensions' code and storage now.
        inbox
            .export("job-1", frozen.clone(), cancel.clone())
            .unwrap();
        // The person edits the project and the extension's storage meanwhile.
        fixture.host.run_command("spy.store", json!(["abcdef"]));
        edit_text(&fixture, "spy.text", "edited meanwhile");
        inbox.ask(1, needs.clone(), json!({"job":"job-1"}));
        let live = fixture.kernel.published("window").unwrap().revision();
        inbox.ask(2, needs.clone(), json!({"revision": live}));
        // The export draws its frozen project and storage; the viewer the edits.
        assert_eq!(
            answer_of(&answered, 1, Duration::from_secs(30)),
            json!(6 + 2)
        );
        assert_eq!(
            answer_of(&answered, 2, Duration::from_secs(30)),
            json!(16 + 6)
        );
        assert_eq!(fixture.host.export_pages().len(), 1);
        // Neither the export realm's write nor its registrations reached the kernel.
        thread::sleep(Duration::from_millis(200));
        assert_ne!(fixture.name(), "written by frozen");
        let commands = fixture
            .kernel
            .state()
            .registries
            .get("commands")
            .map_or(0, Vec::len);
        assert_eq!(commands, 2, "only the window's realm registers");
        // The export ends: its realm and page go, and nothing answers it again.
        cancel.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fixture.host.export_pages().is_empty() {
            assert!(Instant::now() < deadline, "the export realm ended");
            thread::sleep(Duration::from_millis(10));
        }
        inbox.ask(3, needs, json!({"job":"job-1"}));
        thread::sleep(Duration::from_millis(100));
        assert!(!answered.lock().unwrap().iter().any(|(b, _)| *b == 3));
    }
    #[test]
    fn an_export_realms_capture_renders_its_frozen_project_and_asks_it_inline() {
        // The renderer stands in for the app's engine job: it checks which
        // project it was given and asks the realm waiting for it to draw t1.
        type Wiring = Arc<Mutex<Option<(RasterInbox, Answers)>>>;
        let wiring: Wiring = Arc::default();
        let rendered: Arc<Mutex<Vec<u64>>> = Arc::default();
        let (w, r) = (wiring.clone(), rendered.clone());
        let frames: crate::FrameRenderer = Arc::new(move |frame: &crate::FrameRequest| {
            let published = frame
                .published
                .clone()
                .ok_or("a frame of the window's project")?;
            r.lock().unwrap().push(published.revision());
            let (inbox, answered) = w.lock().unwrap().clone().unwrap();
            let comp = published.snapshot.render_comp();
            inbox.export("capture-1", published, Arc::default())?;
            inbox.ask(
                77,
                json!([{"composition":comp,"layer":"t1","seconds":0,"scale":1}]),
                json!({"job":"capture-1"}),
            );
            answer_of(&answered, 77, Duration::from_secs(10));
            let mut png = vec![];
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0, 0, 255, 255])
                .unwrap();
            Ok(png)
        });
        let capturer = "export default function activate(){
            globalThis.capture = () => { globalThis.PM.renderFrameTo(0, 1, 1) } }";
        let (fixture, answered) = export_window(
            &[("spy", SPY), ("capturer", capturer)],
            "frozen",
            Some(frames),
        );
        *wiring.lock().unwrap() = Some((fixture.host.raster_inbox(), answered.clone()));
        let frozen = fixture.kernel.published("window").unwrap();
        let inbox = fixture.host.raster_inbox();
        let cancel = Arc::new(AtomicBool::new(false));
        inbox
            .export("job-2", frozen.clone(), cancel.clone())
            .unwrap();
        edit_text(&fixture, "spy.text", "edited meanwhile");
        inbox.ask(5, needs(&fixture), json!({"job":"job-2"}));
        // The capture inside the export's override drew the export's project,
        // its t1 answered inline by the export realm (frozen text: 6).
        assert_eq!(answer_of(&answered, 5, Duration::from_secs(40)), json!(6));
        assert_eq!(answer_of(&answered, 77, Duration::from_secs(1)), json!(6));
        assert_eq!(*rendered.lock().unwrap(), vec![frozen.revision()]);
        cancel.store(true, Ordering::Release);
    }
    /// A window realm's capture draws the publication the realm holds, so
    /// an edit the kernel takes while the realm waits never leaves it asked
    /// for a revision it cannot reach (it used to retry to the 60 s deadline,
    /// frozen). The renderer stands in for the app's `frame_renderer` and
    /// `JobOverrides`: the kernel's project when given none, a look's exact
    /// ask while that is the kernel's current one, else the job route.
    #[test]
    fn a_window_capture_draws_the_realms_own_revision_when_an_edit_lands_meanwhile() {
        type Wiring = Arc<Mutex<Option<(RasterInbox, Answers, KernelHandle)>>>;
        let wiring: Wiring = Arc::default();
        let rendered: Arc<Mutex<Vec<(u64, u64)>>> = Arc::default();
        let (w, r) = (wiring.clone(), rendered.clone());
        let frames: crate::FrameRenderer = Arc::new(move |frame: &crate::FrameRequest| {
            let (inbox, answered, kernel) = w.lock().unwrap().clone().unwrap();
            let before = kernel.published("window").unwrap();
            // The person renames t1 while the realm waits for this frame.
            let rename = pm_kernel::Call::Transact {
                label: Some("Rename".into()),
                body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::Layer(
                    pm_kernel::LayerCommand::Set {
                        layer_id: "t1".into(),
                        patch: pm_kernel::commands::layers::LayerPatch {
                            name: Some("Renamed meanwhile".into()),
                            ..Default::default()
                        },
                    },
                )]),
                expected_revision: None,
            };
            kernel
                .call_blocking("window", Request::new(pm_kernel::Caller::native(), rename))
                .map_err(|e| e.to_string())?;
            let published = frame
                .published
                .clone()
                .or_else(|| kernel.published("window"))
                .unwrap();
            let now = kernel.published("window").unwrap();
            r.lock()
                .unwrap()
                .push((published.revision(), before.revision()));
            let comp = published.snapshot.render_comp();
            let needs = json!([{"composition":comp,"layer":"t1","seconds":0,"scale":1}]);
            let started = Instant::now();
            for batch in 80.. {
                if Arc::ptr_eq(&now, &published) {
                    let ask = json!({"revision": published.revision(), "exact": true});
                    inbox.ask(batch, needs.clone(), ask);
                } else {
                    inbox.export("capture-1", published.clone(), Arc::default())?;
                    inbox.ask(batch, needs.clone(), json!({"job":"capture-1"}));
                }
                // A number is a drawn width; anything else is "ask again".
                if answer_of(&answered, batch, Duration::from_secs(5)).is_number() {
                    break;
                }
                if started.elapsed() > Duration::from_secs(3) {
                    return Err("the waiting realm never reached the frame's revision".into());
                }
                thread::sleep(Duration::from_millis(20));
            }
            let mut png = vec![];
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[0, 0, 255, 255])
                .unwrap();
            Ok(png)
        });
        let capturer = "export default function activate(){
            globalThis.capture = () => { globalThis.PM.renderFrameTo(0, 1, 1) } }";
        let (fixture, answered) = export_window(
            &[("spy", SPY), ("capturer", capturer)],
            "window",
            Some(frames),
        );
        *wiring.lock().unwrap() = Some((
            fixture.host.raster_inbox(),
            answered.clone(),
            fixture.kernel.clone(),
        ));
        let live = fixture.kernel.published("window").unwrap().revision();
        let started = Instant::now();
        fixture.host.raster_inbox().ask(5, needs(&fixture), json!({"revision": live}));
        // t1's override captured a frame of the realm's own project, its t1
        // answered inline by the waiting realm, and then drew itself.
        assert_eq!(answer_of(&answered, 5, Duration::from_secs(20)), json!(6));
        assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
        let rendered = rendered.lock().unwrap().clone();
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].0, rendered[0].1, "the frame is the realm's revision");
        assert!(
            answered.lock().unwrap().iter().any(|(b, a)| *b >= 80 && *a == json!(6)),
            "the capture's t1 was drawn by the waiting realm"
        );
    }
    #[test]
    fn an_extension_that_runs_in_the_window_but_not_for_the_export_fails_it_by_name() {
        let flaky = "export default function activate(api: any){
            if (api.project.get().layers?.[0]?.d?.text === 'boom') throw Error('boom') }";
        let (fixture, answered) = export_window(&[("spy", SPY), ("flaky", flaky)], "fine", None);
        assert_eq!(fixture.health("flaky"), "ok");
        edit_text(&fixture, "spy.text", "boom");
        let inbox = fixture.host.raster_inbox();
        let frozen = fixture.kernel.published("window").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        inbox.export("job-3", frozen, cancel.clone()).unwrap();
        inbox.ask(1, needs(&fixture), json!({"job":"job-3"}));
        let answer = answer_of(&answered, 1, Duration::from_secs(30));
        assert_eq!(answer, json!("flaky could not start for the export"));
        cancel.store(true, Ordering::Release);
    }
    #[test]
    fn an_export_realm_reads_the_network_but_never_sends_changes() {
        // Activation tries a POST; the realm notes whether it was refused as a
        // delivery, and the override's width says so (3: refused).
        let poster = "export default function activate(api: any){
            globalThis.refused = false;
            fetch('http://127.0.0.1:9/record', {method:'POST', body:'x'})
              .catch((e: any) => { globalThis.refused = /only read the network/.test(String(e?.message ?? e)) });
            api.services.register('raster',()=>{
              const c=document.createElement('canvas'); c.width=globalThis.refused ? 3 : 1; c.height=1;
              return {cv:c,w:c.width,h:1,anchorX:0,anchorY:0} }) }";
        let (fixture, answered) = export_window(&[("poster", poster)], "text", None);
        let inbox = fixture.host.raster_inbox();
        thread::sleep(Duration::from_millis(500));
        let live = fixture.kernel.published("window").unwrap().revision();
        inbox.ask(1, needs(&fixture), json!({"revision": live}));
        assert_eq!(answer_of(&answered, 1, Duration::from_secs(10)), json!(1));
        let cancel = Arc::new(AtomicBool::new(false));
        let frozen = fixture.kernel.published("window").unwrap();
        inbox.export("job-5", frozen, cancel.clone()).unwrap();
        inbox.ask(2, needs(&fixture), json!({"job":"job-5"}));
        assert_eq!(answer_of(&answered, 2, Duration::from_secs(40)), json!(3));
        cancel.store(true, Ordering::Release);
    }
    #[test]
    fn a_layout_read_no_page_answers_fails_the_export() {
        let measure = "export default function activate(api: any){
            const base = api.services.get('raster');
            api.services.register('raster',(L: any, ...rest: any[])=>{
              const d=document.createElement('div'); d.textContent='x'; document.body.append(d);
              const w=d.getBoundingClientRect().width; d.remove();
              const c=document.createElement('canvas'); c.width=1+Math.round(w); c.height=1;
              return {cv:c,w:c.width,h:1,anchorX:0,anchorY:0} }) }";
        let (fixture, answered) = export_window(&[("measure", measure)], "text", None);
        let inbox = fixture.host.raster_inbox();
        let cancel = Arc::new(AtomicBool::new(false));
        let frozen = fixture.kernel.published("window").unwrap();
        inbox.export("job-4", frozen, cancel.clone()).unwrap();
        // No window shows the export's page here: the read cannot be answered.
        inbox.ask(1, needs(&fixture), json!({"job":"job-4"}));
        let answer = answer_of(&answered, 1, Duration::from_secs(40));
        assert!(
            answer.as_str().unwrap().contains("stopped answering"),
            "{answer}"
        );
        cancel.store(true, Ordering::Release);
    }
    #[test]
    fn the_projects_text_faces_and_pm_fonts_reach_the_window() {
        let root = tempfile::tempdir().unwrap();
        extension(
            &root.path().join("extensions"),
            "fonts",
            &[(
                "index.ts",
                "export default function activate(api: any){
                   const fonts = api.media.fonts;
                   api.commands.register({id:'fonts.ensure',label:'Ensure',run(){
                     void fonts.ensure('Probe Serif', 700).then(() => api.storage.set('ensured', true)) }});
                   api.commands.register({id:'fonts.list',label:'List',run(){
                     api.storage.set('seen', {system: fonts.system.length, has: fonts.families.includes('Helvetica')}) }});
                 }",
            )],
        );
        let mut document = crate::initial()["document"].clone();
        document["layers"] = json!([{"id":"t1","type":"text","name":"T1","from":0,"dur":5,
            "d":{"text":"a","font":{"v":"Probe Sans","kf":[]},"weight":{"v":600,"kf":[]}}}]);
        let fixture = Fixture::start_with(root, document, None);
        fixture.until("activation", |host| host.is_ready());
        let mut requests = vec![];
        let mut take = |want: &str| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                requests.extend(fixture.host.take_requests());
                let found = requests
                    .iter()
                    .filter(|r| r["kind"] == "fonts")
                    .flat_map(|r| r["faces"].as_array().cloned().unwrap_or_default())
                    .find(|f| f["family"] == want);
                if let Some(found) = found {
                    return found;
                }
                assert!(
                    Instant::now() < deadline,
                    "{want} was asked for: {requests:?}"
                );
                thread::sleep(Duration::from_millis(10));
            }
        };
        // The project's text face, as its layer animates it (`{v}`).
        assert_eq!(
            take("Probe Sans"),
            json!({"family":"Probe Sans","weight":600,"italic":false})
        );
        fixture.host.run_command("fonts.ensure", json!([]));
        assert_eq!(
            take("Probe Serif"),
            json!({"family":"Probe Serif","weight":700,"italic":false})
        );
        // `ensure` waits for its own face, not for any page answer.
        let ensured = || {
            fixture
                .kernel
                .state()
                .settings
                .get("ext.fonts")
                .is_some_and(|v| pm_model::json::to_canonical_string(v).contains("ensured"))
        };
        fixture.host.fonts_loaded("", json!(["Probe Sans|600|0"]));
        thread::sleep(Duration::from_millis(300));
        assert!(!ensured(), "another face settling does not resolve ensure");
        fixture.host.fonts_loaded("", json!(["Probe Serif|700|0"]));
        fixture.until("ensure resolved", |_| ensured());
        fixture.host.run_command("fonts.list", json!([]));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let stored = fixture.kernel.state().settings.get("ext.fonts").map(|v| {
                serde_json::from_str::<Value>(&pm_model::json::to_canonical_string(v)).unwrap()
            });
            if let Some(seen) = stored.map(|v| v["seen"].clone()).filter(|v| !v.is_null()) {
                assert!(seen["system"].as_u64().unwrap() > 0, "this Mac's families");
                assert_eq!(seen["has"], true);
                break;
            }
            assert!(Instant::now() < deadline, "PM.Fonts answered");
            thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn a_layout_answer_that_comes_before_its_read_waits_is_kept() {
        let mut measures = Measures::default();
        measures.answer(7, &json!({"request":7}));
        assert_eq!(measures.wait(7).ok(), Some(json!({"request":7})));
        // The usual order: the read waits, then the answer comes.
        let rx = measures.wait(8).unwrap_err();
        measures.answer(8, &json!({"request":8}));
        assert_eq!(rx.try_recv().ok(), Some(json!({"request":8})));
        // Answers no read waits for don't pile up.
        for request in 100..120 {
            measures.answer(request, &json!({}));
        }
        assert_eq!(measures.early.len(), 8);
    }
    fn command_of(host: &ExtensionHost, id: &str) -> bool {
        host.contributions().is_some_and(|c| {
            c["commands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|command| command["id"] == id)
        })
    }
    /// A v1–3 theme reaches the native shell as token patches (plan 3.6.5).
    #[test]
    fn an_activated_v1_theme_publishes_native_token_patches() {
        let root = tempfile::tempdir().unwrap();
        let extensions = root.path().join("extensions");
        extension(
            &extensions,
            "synthetic-theme",
            &[(
                "index.ts",
                "export default function(api){
                   api.theme.register({id:'synthetic-theme.warm',name:'Warm',scheme:'dark',
                     tokens:{'--accent':'#123456','--bg-panel':'#202020'},css:''});
                   api.theme.activate('synthetic-theme.warm');
                   api.commands.register({id:'synthetic-theme.off',label:'Off',run(){api.theme.activate('default')}});
                 }",
            )],
        );
        let fixture = Fixture::start(root);
        fixture.until("theme", |host| {
            host.contributions()
                .is_some_and(|c| c["nativeTheme"]["id"] == "synthetic-theme.warm")
        });
        let theme = fixture.host.contributions().unwrap()["nativeTheme"].clone();
        assert_eq!(theme["dark"]["colors.accent"], 0x123456, "{theme}");
        assert_eq!(theme["light"]["colors.accent"], 0x123456, "{theme}");
        // The timeline surface's inputs come with it (surface rules beyond tokens).
        assert_eq!(theme["timeline"]["dark"]["track"], 0x202020, "{theme}");
        assert_eq!(theme["timeline"]["dark"]["accent"], 0x123456, "{theme}");
        assert!(theme["error"].is_null(), "{theme}");
        // Going back to the default theme publishes an empty patch.
        fixture
            .host
            .run_command("synthetic-theme.off", serde_json::json!([]));
        fixture.until("default", |host| {
            host.contributions()
                .is_some_and(|c| c["nativeTheme"]["id"] == "default")
        });
        let theme = fixture.host.contributions().unwrap()["nativeTheme"].clone();
        assert_eq!(theme["dark"], serde_json::json!({}));
    }
    /// v1–3 extensions size docks from `window.innerWidth` on activation, as
    /// in Electron; the realm has the editor window's viewport.
    #[test]
    fn extensions_read_the_editor_window_size() {
        let root = tempfile::tempdir().unwrap();
        let extensions = root.path().join("extensions");
        extension(
            &extensions,
            "synthetic-size",
            &[(
                "index.ts",
                "export default function(api){
                   api.workspace.mutate((w) => { for (const d of w.layout.docks) if (d.id === 'left') d.size = Math.round(window.innerWidth * .18); });
                   api.commands.register({id:'synthetic-size.'+window.innerWidth+'x'+window.innerHeight,label:'Size',run(){}});
                   window.addEventListener('resize', () => api.commands.register({id:'synthetic-size.resized-'+window.innerWidth,label:'Resized',run(){}}));
                 }",
            )],
        );
        let fixture = Fixture::start(root);
        let has = |id: &'static str| {
            move |host: &ExtensionHost| {
                host.contributions().is_some_and(|c| {
                    c["commands"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|command| command["id"] == id)
                })
            }
        };
        fixture.until(
            "activation with the default size",
            has("synthetic-size.1440x900"),
        );
        fixture.host.set_viewport(1200., 700., 2.);
        fixture.until("resize", has("synthetic-size.resized-1200"));
    }
}
