//! Main-thread services for windowless MCP extension realms. The production
//! applier lays out hidden measurement elements; no editor or window is shown.
use objc2::MainThreadMarker;
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
use pm_compat_host::{
    DomPanelHost, HostEvent, MacDomHost, PanelSlot, PanelSpec, Rect, StoreDataStores, SyncHandler,
    Trust,
};
use pm_compat_runner::host::ExtensionHost;
use pm_render::text::host::RenderFonts;
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
static HOSTS: Mutex<Vec<Weak<ExtensionHost>>> = Mutex::new(vec![]);
const PANEL: u64 = 1;
const EXPORT: &str = "export:";

pub(crate) fn watch(host: &Arc<ExtensionHost>) {
    if ACTIVE.load(Ordering::Acquire) {
        HOSTS.lock().unwrap().push(Arc::downgrade(host));
    }
}

struct Feedback {
    events: RefCell<Vec<HostEvent>>,
    messages: mpsc::SyncSender<Value>,
}
impl SyncHandler for Feedback {
    fn event(&self, event: HostEvent) {
        self.events.borrow_mut().push(event);
    }
    fn remote_dom(&self, payload: Value) -> pm_compat_host::Result<()> {
        self.messages
            .try_send(payload)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => pm_compat_host::Error::Backpressure,
                mpsc::TrySendError::Disconnected(_) => pm_compat_host::Error::StaleGeneration,
            })
    }
}
struct Page {
    web: MacDomHost,
    feedback: Rc<Feedback>,
    view: u64,
    ready: bool,
}
impl Page {
    fn new(
        host: &ExtensionHost,
        key: &str,
        view: u64,
        stores: &mut StoreDataStores,
    ) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::sync_channel(128);
        let feedback = Rc::new(Feedback {
            events: RefCell::default(),
            messages: tx,
        });
        let sink: Box<dyn Fn(Value) + Send> = match key.strip_prefix(EXPORT) {
            Some(job) => Box::new(host.export_page_sink(job)),
            None => Box::new(host.page_sink(view)),
        };
        std::thread::spawn(move || {
            while let Ok(message) = rx.recv() {
                sink(message);
            }
        });
        let mut web = MacDomHost::new_remote_dom(
            MainThreadMarker::new().expect("MCP pages stay on main"),
            Trust::Trusted,
            feedback.clone(),
            stores,
            &pm_compat_runner::page_bundle(false, false, false),
        )?;
        web.create(PanelSpec {
            id: PANEL,
            label: "Extension measurements".into(),
        })?;
        // Unattached remote-DOM panels have display:none. A windowless page
        // needs a real layout slot too, even though its NSView is never shown.
        let viewport = Rect::new(0., 0., 1280., 720.);
        web.attach(
            PANEL,
            PanelSlot {
                bounds: viewport,
                clip: viewport,
            },
        )?;
        web.present(viewport)?;
        // Disables WebKit's hidden-page throttling, without parking, attaching,
        // ordering or activating a window.
        web.begin_background_job(false)?;
        Ok(Self {
            web,
            feedback,
            view,
            ready: false,
        })
    }
}
impl Drop for Page {
    fn drop(&mut self) {
        self.web.end_background_job();
    }
}

struct Faces {
    keys: Vec<String>,
    faces: Vec<pm_compat_host::PageFace>,
}
struct Owner {
    host: Weak<ExtensionHost>,
    pages: BTreeMap<String, Page>,
    stores: StoreDataStores,
    wanted: BTreeSet<(String, u16, bool)>,
    faces: Vec<pm_compat_host::PageFace>,
    unavailable: BTreeSet<String>,
    fonts: mpsc::Sender<Faces>,
    resolved: mpsc::Receiver<Faces>,
}
impl Owner {
    fn new(host: Weak<ExtensionHost>) -> Self {
        let (fonts, resolved) = mpsc::channel();
        Self {
            host,
            pages: BTreeMap::new(),
            stores: StoreDataStores::default(),
            wanted: BTreeSet::new(),
            faces: vec![],
            unavailable: BTreeSet::new(),
            fonts,
            resolved,
        }
    }
    fn tick(&mut self) -> bool {
        let Some(host) = self.host.upgrade() else {
            return false;
        };
        let mut running: BTreeMap<String, u64> = host
            .export_pages()
            .into_iter()
            .map(|(job, view)| (format!("{EXPORT}{job}"), view))
            .collect();
        if host.view() != 0 {
            running.insert(String::new(), host.view());
        }
        self.pages
            .retain(|key, page| running.get(key) == Some(&page.view));
        for (key, view) in running {
            if !self.pages.contains_key(&key) {
                match Page::new(&host, &key, view, &mut self.stores) {
                    Ok(mut page) => {
                        let _ = page.web.add_fonts(&self.faces);
                        host.fonts_loaded(
                            &key,
                            self.unavailable
                                .iter()
                                .cloned()
                                .map(Value::String)
                                .collect(),
                        );
                        self.pages.insert(key, page);
                    }
                    Err(error) => eprintln!("MCP extension layout page: {error}"),
                }
            }
        }
        // Font resolution can ask CoreText on main. Resolve on a worker while
        // the main loop continues serving that same broker and WebKit.
        for request in host.take_requests() {
            if request["kind"] != "fonts" {
                continue;
            }
            let wanted: Vec<_> = request["faces"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|face| {
                    let family = face["family"].as_str()?.trim();
                    (!family.is_empty() && family.len() <= 256).then(|| {
                        (
                            family.to_owned(),
                            face["weight"].as_f64().unwrap_or(400.).clamp(1., 1000.) as u16,
                            face["italic"] == true,
                        )
                    })
                })
                .filter(|key| self.wanted.insert(key.clone()))
                .take(256)
                .collect();
            if !wanted.is_empty() {
                let tx = self.fonts.clone();
                std::thread::spawn(move || {
                    let keys = wanted
                        .iter()
                        .map(|(f, w, i)| format!("{f}|{w}|{}", u8::from(*i)))
                        .collect();
                    let _ = tx.send(Faces {
                        keys,
                        faces: crate::text_host::page_faces(&wanted),
                    });
                });
            }
        }
        for resolved in self.resolved.try_iter() {
            let unavailable: Vec<Value> = resolved
                .keys
                .iter()
                .filter(|key| !resolved.faces.iter().any(|f| f.key() == **key))
                .map(|key| key.clone().into())
                .collect();
            self.unavailable.extend(
                unavailable
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned),
            );
            for (key, page) in &mut self.pages {
                let _ = page.web.add_fonts(&resolved.faces);
                host.fonts_loaded(key, unavailable.clone().into());
            }
            self.faces.extend(resolved.faces);
        }
        for (key, page) in &mut self.pages {
            for event in page.feedback.events.borrow_mut().drain(..) {
                match event {
                    HostEvent::Ready { .. } => page.ready = true,
                    HostEvent::ProcessTerminated => page.ready = false,
                    HostEvent::FontsLoaded(keys) => host.fonts_loaded(key, keys.into()),
                    HostEvent::Failure(code) => eprintln!("MCP extension page: {code}"),
                }
            }
            if !page.ready {
                continue;
            }
            let job = key.strip_prefix(EXPORT);
            let mut batches = match job {
                Some(job) => host.take_export_batches(job),
                None => host.take_batches(page.view),
            };
            while let Some(batch) = batches.front() {
                if batch.present(&mut page.web).is_err() {
                    break;
                }
                batches.pop_front();
            }
            match job {
                Some(job) => host.return_export_batches(job, batches),
                None => {
                    host.return_batches(page.view, batches);
                    // Measurement-only DOM still acknowledges any canvas nodes.
                    for canvas in host.take_canvases() {
                        let _ = page.web.present_canvas(PANEL, &canvas);
                    }
                }
            }
        }
        true
    }
}

pub(crate) fn run(
    work: impl FnOnce(RenderFonts) -> anyhow::Result<()> + Send + 'static,
) -> anyhow::Result<()> {
    let mtm =
        MainThreadMarker::new().ok_or_else(|| anyhow::anyhow!("MCP services require main"))?;
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Prohibited);
    app.finishLaunching();
    let mut service = pm_text::host::FontService::new().map_err(anyhow::Error::msg)?;
    let broker = pm_text::host::FontBroker::new();
    broker.set_generation(service.generation());
    // The bounded main-loop turn below admits and serves every font ask.
    broker.set_wake(Arc::new(|| {}));
    let fonts = RenderFonts::new(broker.clone());
    ACTIVE.store(true, Ordering::Release);
    let worker = std::thread::spawn(move || work(fonts));
    let mut owners = vec![];
    while !worker.is_finished() {
        objc2::rc::autoreleasepool(|_| {
            pm_text::host::serve(&mut service, broker.take_asks());
            owners.extend(HOSTS.lock().unwrap().drain(..).map(Owner::new));
            owners.retain_mut(Owner::tick);
            NSRunLoop::mainRunLoop().runMode_beforeDate(
                unsafe { NSDefaultRunLoopMode },
                &NSDate::dateWithTimeIntervalSinceNow(0.01),
            );
        });
    }
    ACTIVE.store(false, Ordering::Release);
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("MCP worker panicked"))?
}
