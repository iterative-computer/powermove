//! An export's own realm (plan 3.2.2: an export pins one generation per
//! extension for the whole job). The window's trusted extensions, as compiled
//! when the export asks for its first override, run again in a second isolate
//! whose project is the export's frozen publication, so the person keeps
//! editing while it renders and nothing they do reaches the export. The live
//! realm never answers an export. The realm has its own hidden page in the
//! window, so the layout reads overrides make (a caret, a label) are real.
//! It ends with its export (finished, failed or cancelled) or its window.
use super::{Measures, PAGE_QUEUE, PAGE_SESSION, PANEL};
use super::{Shared, TrySink};
use crate::recorder::{CanvasText, NativeHost, RealmEvents};
use crate::{RasterAnswer, RasterHooks};
use pm_compat_host::{AdmittedDomBatch, RemoteDomPanel};
use pm_kernel::{KernelHandle, Published};
use pm_remote_dom::protocol::Session;
use pm_runtime_v8::{Realm, RealmKind, RuntimePool};
use serde_json::Value;
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long an export realm's first answers wait for its page to start
/// reading, so the first layout read is real. Without a page (no window can
/// show one) the realm answers after this and its reads measure nothing.
const PAGE_WAIT: Duration = Duration::from_secs(8);
/// How long a layout read waits for the page. An export waits for its
/// answers (the viewer's realm gives up after 250 ms and draws on): a read
/// answered with nothing is remembered by the extension as the layout.
const READ_WAIT: Duration = Duration::from_secs(10);

pub(super) enum RunInput {
    /// A batch of an export frame's needs: `[{composition, layer, seconds, scale}]`.
    Ask(u64, Value),
    Page(Value),
    Dom,
    Booted,
    Unresponsive(u64),
    FontsLoaded,
}

/// The export realms of one window, by job; the app shows each one's page.
#[derive(Default)]
pub(super) struct Exports {
    runs: std::collections::BTreeMap<String, Route>,
}
pub(super) struct Route {
    pub(super) inbox: Sender<RunInput>,
    pub(super) view: u64,
    pub(super) batches: VecDeque<AdmittedDomBatch>,
}
impl Exports {
    pub(super) fn inbox(&self, job: &str) -> Option<Sender<RunInput>> {
        self.runs.get(job).map(|r| r.inbox.clone())
    }
    pub(super) fn insert(&mut self, job: String, route: Route) {
        self.runs.insert(job, route);
    }
    pub(super) fn remove(&mut self, job: &str) {
        self.runs.remove(job);
    }
    pub(super) fn clear(&mut self) {
        self.runs.clear();
    }
    pub(super) fn pages(&self) -> Vec<(String, u64)> {
        self.runs.iter().map(|(j, r)| (j.clone(), r.view)).collect()
    }
    pub(super) fn batches(&mut self, job: &str) -> Option<&mut VecDeque<AdmittedDomBatch>> {
        self.runs.get_mut(job).map(|r| &mut r.batches)
    }
}
static NEXT_VIEW: AtomicU64 = AtomicU64::new(1 << 32);
pub(super) fn next_view() -> u64 {
    NEXT_VIEW.fetch_add(1, Ordering::Relaxed)
}

/// What an export realm starts from: the live window's plan and the frozen project.
pub(super) struct Start {
    pub(super) job: String,
    pub(super) frozen: Arc<Published>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) records: Value,
    pub(super) viewport: Option<(f32, f32, f32)>,
    pub(super) text: Option<CanvasText>,
    pub(super) rasters: Option<RasterHooks>,
    pub(super) kernel: KernelHandle,
    pub(super) shared: Arc<Shared>,
    /// Set when the window's extensions stop.
    pub(super) closing: Arc<AtomicBool>,
}

struct Events {
    inbox: Sender<RunInput>,
    shared: Arc<Shared>,
    measuring: Mutex<Measures>,
    /// The page has answered once; a missed first read waits no more.
    reading: AtomicBool,
    page_missed: AtomicBool,
    dom_pending: AtomicBool,
    /// Extensions that did not start here, by id.
    failed: Mutex<BTreeSet<String>>,
}
impl RealmEvents for Arc<Events> {
    fn health(&self, id: &str, health: &Value) {
        if matches!(
            health["state"].as_str(),
            Some("activation-error" | "runtime-error")
        ) {
            self.failed.lock().unwrap().insert(id.to_owned());
        }
    }
    /// The faces its project's text uses reach the window like the live
    /// realm's; nothing else this realm asks of the window is shown.
    fn request(&self, request: Value) {
        if request["kind"] == "fonts" {
            self.shared.update(|p| p.requests.push(request));
        }
    }
    fn dom_ready(&self) {
        if !self.dom_pending.swap(true, Ordering::AcqRel) {
            let _ = self.inbox.send(RunInput::Dom);
        }
    }
    fn ready(&self) {
        let _ = self.inbox.send(RunInput::Booted);
    }
    fn unresponsive(&self, execution: u64) {
        let _ = self.inbox.send(RunInput::Unresponsive(execution));
    }
    fn measure_sync(&self, request: u64) -> Option<Value> {
        let rx = match self.measuring.lock().unwrap().wait(request) {
            Ok(answer) => return Some(answer),
            Err(rx) => rx,
        };
        self.dom_ready();
        // Without a page (it never started reading) nothing waits.
        let wait =
            if self.page_missed.load(Ordering::Acquire) && !self.reading.load(Ordering::Acquire) {
                Duration::ZERO
            } else {
                READ_WAIT
            };
        let answer = rx.recv_timeout(wait).ok();

        self.measuring.lock().unwrap().waiting = None;
        answer
    }
}

struct PageQueue<'a>(&'a Shared, &'a str);
impl TrySink<AdmittedDomBatch> for PageQueue<'_> {
    fn try_send(
        &mut self,
        batch: AdmittedDomBatch,
    ) -> std::result::Result<(), pm_remote_dom::Error> {
        let mut exports = self.0.exports.lock().unwrap();
        let queue = exports
            .batches(self.1)
            .ok_or(pm_remote_dom::Error::Backpressure)?;
        if queue.len() >= PAGE_QUEUE {
            return Err(pm_remote_dom::Error::Backpressure);
        }
        queue.push_back(batch);
        Ok(())
    }
}

struct Run {
    start: Start,
    events: Arc<Events>,
    live: Option<(Realm, RuntimePool, Arc<NativeHost>)>,
    router: Option<RemoteDomPanel>,
    dom_waiting: VecDeque<Value>,
    /// When every extension had activated.
    booted: Option<Instant>,
    /// Why this realm cannot answer: every ask gets it.
    failure: Option<String>,
    /// Asks until the realm is ready, in order.
    waiting: VecDeque<(u64, Value)>,
    /// Sources the realm's queue had no room for yet, in order.
    queue: VecDeque<String>,
    producing: Arc<AtomicBool>,
}

/// Runs one export realm on the calling thread until its export or window ends.
pub(super) fn run(start: Start, inbox: Sender<RunInput>, input: Receiver<RunInput>) {
    let events = Arc::new(Events {
        inbox,
        shared: start.shared.clone(),
        measuring: Mutex::new(Measures {
            waiting: None,
            early: VecDeque::new(),
        }),
        reading: AtomicBool::new(false),
        page_missed: AtomicBool::new(false),
        dom_pending: AtomicBool::new(false),
        failed: Mutex::default(),
    });
    let mut run = Run {
        start,
        events,
        live: None,
        router: None,
        dom_waiting: VecDeque::new(),
        booted: None,
        failure: None,
        waiting: VecDeque::new(),
        queue: VecDeque::new(),
        producing: Arc::new(AtomicBool::new(false)),
    };

    if let Err(error) = run.boot() {
        run.failure = Some(format!("The export's extensions could not start: {error}"));
    }
    loop {
        if run.start.cancel.load(Ordering::Acquire) || run.start.closing.load(Ordering::Acquire) {
            break;
        }
        let wait = if run.queue.is_empty() {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(2)
        };
        match input.recv_timeout(wait) {
            Ok(RunInput::Ask(batch, needs)) => run.ask(batch, needs),
            Ok(RunInput::Booted) => run.booted(),
            Ok(RunInput::Dom) => run.drain_dom(),
            Ok(RunInput::Page(message)) => run.page(message),
            Ok(RunInput::Unresponsive(execution)) => run.unresponsive(execution),
            Ok(RunInput::FontsLoaded) => run.eval("__compat.fontsLoaded()".into()),
            Err(RecvTimeoutError::Timeout) => (),
            Err(RecvTimeoutError::Disconnected) => break,
        }
        run.dispatch();
    }
    if let Some((realm, ..)) = run.live.take() {
        realm.close();
    }
    run.start
        .shared
        .exports
        .lock()
        .unwrap()
        .remove(&run.start.job);
    (run.start.shared.wake)();
}

impl Run {
    fn boot(&mut self) -> pm_runtime_v8::Result<()> {
        let adapter = crate::adapter::KernelAdapter::frozen(
            self.start.kernel.clone(),
            self.start.frozen.clone(),
            crate::PROJECT,
        );
        let host = Arc::new(NativeHost::new(
            adapter,
            crate::transport::Transport::new(None),
            Box::new(self.events.clone()),
        ));
        if let Some(text) = self.start.text.take() {
            host.set_text(text);
        }
        // Answers go to the export's wait; this realm's wrappers never change
        // what the window's viewer or other exports see.
        let producing = self.producing.clone();
        host.set_rasters(self.start.rasters.clone().map(|hooks| RasterHooks {
            producing: Arc::new(move |on, _| producing.store(on, Ordering::Release)),
            ..hooks
        }));
        let pool = crate::pool(&format!("export:{}", self.start.job), host.clone())?;
        let realm = pool.realm(RealmKind::Compat)?;
        crate::boot_realm(&realm, &host, "")?;
        self.router = Some(
            RemoteDomPanel::new(
                PANEL,
                Session {
                    panel: "compat".into(),
                    generation: PAGE_SESSION,
                },
            )
            .map_err(|e| pm_runtime_v8::Error::Host(format!("{e:?}")))?,
        );
        if let Some((width, height, scale)) = self.start.viewport {
            realm.eval(format!("__compat.viewport({width},{height},{scale})"))?;
        }
        realm.eval(format!("__compat.boot({})", self.start.records))?;
        self.live = Some((realm, pool, host));
        Ok(())
    }
    fn fail(&self, batch: u64, needs: &Value, error: &str) {
        let count = needs.as_array().map_or(0, Vec::len);
        if let Some(hooks) = &self.start.rasters {
            (hooks.answers)(
                batch,
                (0..count)
                    .map(|_| RasterAnswer::Failed(error.to_owned()))
                    .collect(),
            );
        }
    }
    /// Booted, and its page is reading (or will not come).
    fn ready(&self) -> bool {
        let Some(booted) = self.booted else {
            return false;
        };
        if self.events.reading.load(Ordering::Acquire) {
            return true;
        }
        if booted.elapsed() < PAGE_WAIT {
            return false;
        }
        self.events.page_missed.store(true, Ordering::Release);
        true
    }
    fn ask(&mut self, batch: u64, needs: Value) {
        self.waiting.push_back((batch, needs));
        self.dispatch();
    }
    fn eval(&mut self, source: String) {
        self.queue.push_back(source);
        self.dispatch();
    }
    /// Asks go to the realm once it is ready; what its queue has no room for
    /// waits here in order (a busy realm is not a stopped one).
    fn dispatch(&mut self) {
        if let Some(error) = self.failure.clone() {
            for (batch, needs) in std::mem::take(&mut self.waiting) {
                self.fail(batch, &needs, &error);
            }
            return;
        }
        if self.ready() {
            for (batch, needs) in std::mem::take(&mut self.waiting) {
                self.queue
                    .push_back(format!("__compat.answerRasters({batch},{needs},{{}})"));
            }
        }
        let Some((realm, ..)) = &self.live else {
            return;
        };
        while let Some(source) = self.queue.front() {
            match realm.eval(source.as_str()) {
                Ok(_) => {
                    self.queue.pop_front();
                }
                Err(pm_runtime_v8::Error::Busy) => return,
                Err(_) => {
                    // Asks already queued wait for answers that will not
                    // come: the export's wait ends them.
                    self.queue.clear();
                    self.failure = Some("The export's extensions stopped".into());
                    return;
                }
            }
        }
    }
    /// Every extension has activated. The window's overrides are registered
    /// here too, or the export fails naming what did not start: drawing those
    /// layers without them would be a wrong export.
    fn booted(&mut self) {
        self.booted = Some(Instant::now());
        if !self.producing.load(Ordering::Acquire) {
            let failed = self.events.failed.lock().unwrap();
            let records = self.start.records.as_array().cloned().unwrap_or_default();
            let name = |id: &String| {
                records
                    .iter()
                    .find(|r| r["id"] == id.as_str())
                    .and_then(|r| r["manifest"]["name"].as_str())
                    .unwrap_or(id)
                    .to_owned()
            };
            let names = failed.iter().map(name).collect::<Vec<_>>().join(", ");
            self.failure = Some(if names.is_empty() {
                "The extensions that draw this export's layers did not start for it".into()
            } else {
                format!("{names} could not start for the export")
            });
        }
    }
    /// An override that runs away fails the export by name; the window's
    /// realm and the person's editing are not involved.
    fn unresponsive(&mut self, execution: u64) {
        let Some((realm, ..)) = &self.live else {
            return;
        };
        let culprit = realm.culprit(execution);
        realm.stop(execution);
        let records = self.start.records.as_array().cloned().unwrap_or_default();
        let name = culprit.and_then(|id| {
            records.iter().find(|r| r["id"] == id.as_str()).map(|r| {
                r["manifest"]["name"]
                    .as_str()
                    .unwrap_or(id.as_str())
                    .to_owned()
            })
        });
        self.failure = Some(match name {
            Some(name) => format!("{name} stopped responding while drawing the export"),
            None => "An extension stopped responding while drawing the export".into(),
        });
    }
    fn page(&mut self, message: Value) {
        let (Some(router), Some(_)) = (&mut self.router, &self.live) else {
            return;
        };
        self.events.reading.store(true, Ordering::Release);
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
            if message["kind"] == "layout"
                && let Some(request) = message["request"].as_u64()
            {
                self.events
                    .measuring
                    .lock()
                    .unwrap()
                    .answer(request, &message);
            }
            self.queue.push_back(format!(
                "__compat.dom.receive({message});__compat.dom.flush()"
            ));
        }
        self.dispatch();
        self.drain_dom();
    }
    fn drain_dom(&mut self) {
        let (Some(router), Some((_, _, host))) = (&mut self.router, &self.live) else {
            return;
        };
        self.events.dom_pending.store(false, Ordering::Release);
        let mut queue = PageQueue(&self.start.shared, &self.start.job);
        let mut admitted = false;
        let mut full = false;
        loop {
            if self.dom_waiting.is_empty() {
                self.dom_waiting.extend(host.take_dom_batches());
            }
            let Some(batch) = self.dom_waiting.front() else {
                break;
            };
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
            (self.start.shared.wake)();
        }
        if !full {
            self.eval("__compat.flushCanvases?.()".into());
        }
    }
}
