//! The running app's v1–3 extensions. `ExtensionHost` owns V8 on its own thread;
//! this side only reads published copies, routes page messages and places the
//! one WebKit document below GPUI. Nothing here can wait on extension code.
use crate::session::Controller;
use gpui::{prelude::*, *};
use objc2::MainThreadMarker;
use pm_compat_host::{
    DomPanelHost, Error as PageError, HostEvent, MacDomHost, PanelSlot, PanelSpec, Rect,
    Result as PageResult, StoreDataStores, SyncHandler, Trust,
};
use pm_compat_runner::{
    host::{ExtensionHost, HostConfig},
    transport::{TransportOwner, TransportRequest},
};
use pm_engine_service::resources::ResourceSnapshot;
use pm_protocol::{clock::TransportCommand, common::PlaybackRate};
use pm_shell::{Command as ShellCommand, MenuEntry, keyboard::Binding, layout, transient::Choice};
use pm_text::host::BrokerResolver;
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
};

const PAGE_PANEL: u64 = 1;
/// The hidden page of an export's own realm (`compat-runner/src/host/export.rs`).
const EXPORT_PAGE: &str = "export:";
const COMMAND: &str = "extension.command:";
const MENU: &str = "extension.menu:";
const STORE: &str = "extension.store";
const INSTALL: &str = "extension.install:";
const UNDO: &str = "extension.undo";
/// Longer than the realm's own deadline; a hung realm never holds a menu back.
/// How long a reloading extension may take to hand over `api.hot.data`.
const HOT_HANDOFF: std::time::Duration = std::time::Duration::from_millis(500);
const MENU_DEADLINE: std::time::Duration = std::time::Duration::from_millis(150);
const OWNER: &str = "extension:";
/// Commands v4 extensions register at run time: `extension.v4:<panel>:<id>`.
const V4_COMMAND: &str = "extension.v4:";
const V4_OWNER: &str = "v4-extension:";
/// `extension.v4theme:<panel>:<id>`, or `…:default` for the built-in look.
const V4_THEME: &str = "extension.v4theme:";
/// `extension.choice:<token>:<index>`: a built-in row of a menu that also
/// carries extension items; its own callback runs.
const CHOICE: &str = "extension.choice:";
/// `extension.v4menu:<panel>:<key>`: a v4 extension's context-menu item.
const V4_MENU: &str = "extension.v4menu:";
thread_local! {
    /// The v4 theme the person chose (`panel:id`), for every window.
    static ACTIVE_THEME: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// The active v1–3 theme last applied, for every window: its translation
    /// (as JSON text) and its dark and light patches. A v4 theme going away
    /// shows this again.
    static V1_THEME: std::cell::RefCell<Option<(String, Patches)>> = const { std::cell::RefCell::new(None) };
}
/// A theme's patches for dark and for light windows.
type Patches = (pm_shell::theme::Patch, pm_shell::theme::Patch);
/// Page messages enter here on AppKit's main thread, so this only enqueues.
struct PageEvents {
    host: Arc<Mutex<Option<std::sync::mpsc::Sender<Value>>>>,
    events: RefCell<Vec<HostEvent>>,
    wake: Rc<dyn Fn()>,
}
impl SyncHandler for PageEvents {
    fn event(&self, event: HostEvent) {
        self.events.borrow_mut().push(event);
        (self.wake)();
    }
    fn remote_dom(&self, payload: Value) -> PageResult<()> {
        match &*self.host.lock().unwrap() {
            Some(sender) if sender.send(payload).is_ok() => Ok(()),
            _ => Err(PageError::StaleGeneration),
        }
    }
}

struct Page {
    host: Rc<RefCell<MacDomHost>>,
    events: Rc<PageEvents>,
    ready: bool,
    /// The run this page belongs to: a realm generation, or a Store helper's
    /// writer. Batches of any other run never reach it.
    view: u64,
}

/// The realm asks the one clock owner to move; the Controller answers on main.
struct Clock(async_channel::Sender<TransportRequest>);
impl TransportOwner for Clock {
    fn request(&self, request: TransportRequest) {
        let _ = self.0.try_send(request);
    }
}

/// A tab menu waiting for the items extensions add to it.
struct OpenMenu {
    key: u64,
    entries: Vec<MenuEntry>,
    at: (f32, f32),
}

/// What an extension's menu items are in the native menu.
fn menu_entries(items: &[Value], token: u64) -> Vec<MenuEntry> {
    items
        .iter()
        .map(|item| {
            if item["separator"] == true {
                return MenuEntry::separator();
            }
            let label = item["label"].as_str().unwrap_or_default();
            if let Some(children) = item["submenu"].as_array() {
                return MenuEntry::submenu(label, menu_entries(children, token));
            }
            let command = item["run"]
                .as_u64()
                .map_or(String::new(), |run| format!("{MENU}{token}:{run}"));
            let mut entry = MenuEntry::item(label, command).checked(item["on"] == true);
            entry.enabled = item["disabled"] != true && item["run"].is_u64();
            entry
        })
        .collect()
}

/// One Store listing per row; choosing it installs that release.
fn store_entries(items: &[Value]) -> Vec<MenuEntry> {
    items
        .iter()
        .filter_map(|item| {
            let (release, repo) = (item["release"].as_str()?, item["repo"].as_str()?);
            let label = item["label"].as_str().unwrap_or(repo);
            Some(MenuEntry::item(label, format!("{INSTALL}{release}:{repo}")))
        })
        .collect()
}

/// Only web links leave the app; nothing runs a local or custom scheme.
fn external_link(url: &str) -> bool {
    url.len() <= 2048 && (url.starts_with("https://") || url.starts_with("http://"))
}

pub struct Extensions {
    /// Panels this window's extensions contribute: id and title, in menu order.
    pub contributed: Vec<(String, String)>,
    pub host: Option<ExtensionHost>,
    /// This window's raster overrides; dropped with its host.
    pub(crate) overrides: Option<crate::raster_overrides::Overrides>,
    /// The document whose captures carry this window's definitions.
    doc_id: Option<u64>,
    /// One WebKit document per trust domain: the trusted realm's under the empty
    /// key, then each Store extension's own. Never shared.
    pages: BTreeMap<String, Page>,
    views: Rc<RefCell<BTreeMap<String, Rc<RefCell<MacDomHost>>>>>,
    data_stores: StoreDataStores,
    /// Panels drawn by a Store helper, with its id and run.
    stores: BTreeMap<String, (String, u64)>,
    owners: Rc<RefCell<BTreeMap<String, String>>>,
    /// Open contributed panels and where the shell placed their bodies.
    panels: BTreeMap<String, layout::Rect>,
    known_panels: BTreeSet<String>,
    /// Panels whose definition went away while open. They return with it.
    suspended: BTreeSet<String>,
    open_panels: BTreeSet<String>,
    /// Open panels with no body right now: tabs behind another.
    hidden: BTreeSet<String>,
    menu: Option<OpenMenu>,
    /// A built-in menu's callback while extensions answer for its menu.
    choice: Option<(u64, crate::timeline_surface::Choose)>,
    next_menu: u64,
    placed: BTreeMap<String, layout::Rect>,
    seen: u64,
    commands: BTreeSet<String>,
    /// Palette entries from v1–3 contributions and from v4 `commands.register`,
    /// and the v4 registry revision they reflect.
    compat_choices: Vec<Choice>,
    v4_choices: Vec<Choice>,
    v4_seen: u64,
    v4_theme_choices: Vec<Choice>,
    /// The active v1–3 theme's timeline surface inputs, per scheme.
    pub v1_timeline: Value,
    /// The viewport last sent to the compat realm.
    viewport: Option<(f32, f32, f32)>,
    /// The `themes.activate` request this window last acted on (its number).
    v4_theme_requested: Option<u64>,
    v4_themes: Vec<Value>,
    v4_themes_seen: u64,
    /// Mounted v4 surface extensions by title, with the source they run.
    surfaces: BTreeMap<String, (String, String)>,
    /// Forks drawing a built-in panel in its place: id to (panel, source).
    forks: BTreeMap<String, (String, String)>,
    /// Surface ids this window has mounted, and whether the first scan is done:
    /// a panel published while the window is open is shown, not left in a menu.
    mounted_once: BTreeSet<String>,
    booted: bool,
    /// The D11 sheet is up for this task.
    unresponsive: Option<u64>,
    /// Quit stopped this window's extensions (a hung one); a cancelled quit
    /// starts them again.
    pub stopped_for_quit: bool,
    /// The engine's faces for the project's text, given to every page.
    faces: Vec<pm_compat_host::PageFace>,
    /// Faces asked for, resolved or not: each is resolved once.
    face_keys: BTreeSet<(String, u16, bool)>,
    /// Contributed definitions, pinned per generation for viewers and exports.
    pub resources: Arc<ResourceSnapshot>,
    /// Where the realm's viewer stage shows the page over the viewer, once an
    /// extension made one (`viewer.stage`, `viewer.ov`). The viewer keeps the
    /// pointer there.
    stage: Rc<std::cell::Cell<Option<layout::Rect>>>,
    stage_used: bool,
    /// The viewer slot and composition frame last told to the realm.
    stage_sent: Option<Option<[i64; 8]>>,
    /// The viewer frame last told to the realm (its stamp's sequence).
    presented: Option<u64>,
}
impl Default for Extensions {
    fn default() -> Self {
        Self {
            contributed: Vec::new(),
            host: None,
            overrides: None,
            doc_id: None,
            pages: BTreeMap::new(),
            views: Rc::default(),
            data_stores: StoreDataStores::default(),
            stores: BTreeMap::new(),
            owners: Rc::default(),
            panels: BTreeMap::new(),
            known_panels: BTreeSet::new(),
            suspended: BTreeSet::new(),
            open_panels: BTreeSet::new(),
            hidden: BTreeSet::new(),
            menu: None,
            choice: None,
            next_menu: 0,
            placed: BTreeMap::new(),
            seen: 0,
            commands: BTreeSet::new(),
            compat_choices: vec![],
            v4_choices: vec![],
            v4_seen: 0,
            v4_theme_choices: vec![],
            v4_theme_requested: None,
            viewport: None,
            v1_timeline: Value::Null,
            v4_themes: vec![],
            v4_themes_seen: 0,
            faces: vec![],
            face_keys: BTreeSet::new(),
            surfaces: BTreeMap::new(),
            forks: BTreeMap::new(),
            mounted_once: BTreeSet::new(),
            booted: false,
            unresponsive: None,
            stopped_for_quit: false,
            resources: Arc::default(),
            stage: Rc::default(),
            stage_used: false,
            stage_sent: None,
            presented: None,
        }
    }
}
impl Drop for Extensions {
    fn drop(&mut self) {
        if let Some(doc_id) = self.doc_id {
            crate::mcp_services::set_resources(doc_id, None);
        }
    }
}
impl Extensions {
    fn palette_choices(&self) -> Vec<Choice> {
        self.compat_choices
            .iter()
            .chain(&self.v4_choices)
            .chain(&self.v4_theme_choices)
            .cloned()
            .collect()
    }
    pub fn owns_panel(&self, id: &str) -> bool {
        self.known_panels.contains(id)
    }
}
/// The shipped app's extensions as first launch migrated them (its sealed
/// checkpoint, kept current by each launch's delta pass), then this profile's
/// own. The native folder wins when both have an id, and is the one agents
/// publish into (last). The shipped app's live folder is never read: an
/// isolated profile stays isolated.
pub(crate) fn roots(profile: &std::path::Path) -> Vec<PathBuf> {
    vec![
        profile.join("extensions-shared"),
        profile.join("extensions"),
    ]
}
impl Controller {
    /// Quitting while an extension isn't responding (D11): its sheet goes,
    /// the realm is stopped without a restart (its open entry is sealed;
    /// acknowledged kernel edits stay) and the quit goes on. `false` when no
    /// extension in this window is hung.
    pub fn stop_hung_extensions_for_quit(&mut self) -> bool {
        if self.extensions.unresponsive.is_none() || self.extensions.host.is_none() {
            return false;
        }
        {
            let shell = self.host.borrow();
            if let Some(sheet) = shell.window.attachedSheet() {
                shell
                    .window
                    .endSheet_returnCode(&sheet, objc2_app_kit::NSModalResponseAbort);
            }
        }
        let mut shell = self.host.borrow_mut();
        shell.native_present = None;
        shell.native_accessibility = vec![];
        drop(shell);
        // Dropping the host closes the realm off the main thread.
        self.reset_extensions();
        self.extensions.stopped_for_quit = true;
        true
    }
    /// A fresh extension state for this window. Resource generations keep
    /// counting up, so the viewer takes the next definitions as newer.
    fn reset_extensions(&mut self) {
        let generation = self.extensions.resources.generation;
        self.extensions = Extensions::default();
        self.extensions.resources = Arc::new(ResourceSnapshot {
            generation,
            ..Default::default()
        });
    }
    pub fn start_extensions(&mut self, cx: &mut Context<Self>) {
        let mut shell = self.host.borrow_mut();
        shell.native_present = None;
        shell.native_accessibility = vec![];
        drop(shell);
        self.reset_extensions();
        let Some(session) = &self.session else {
            return;
        };
        let project = session.project.clone();
        let (wake_tx, wake_rx) = async_channel::bounded(1);
        let (clock_tx, clock_rx) = async_channel::bounded(64);
        let doc_id = self.kernel.published(&project).map(|p| p.snapshot.doc_id);
        let rasters = crate::raster_overrides::Overrides::new(
            &self.engine,
            self.kernel.clone(),
            project.clone(),
            self.job_overrides.clone(),
        );
        let frames = crate::mcp_services::frame_renderer(
            self.kernel.clone(),
            project.clone(),
            self.profile.clone(),
            self.job_overrides.clone(),
        )
        .ok();
        let host = ExtensionHost::start(HostConfig {
            kernel: self.kernel.clone(),
            frames,
            rasters: Some(rasters.hooks()),
            project,
            roots: roots(&self.profile),
            cache: self.profile.join("extensions-build"),
            owner: Some(Arc::new(Clock(clock_tx))),
            text: self.font_broker.clone().map(|broker| {
                Box::new(move || {
                    let resolver = BrokerResolver::new(broker.clone(), Default::default());
                    Box::new(pm_recorder::TextEngine::new(resolver)) as pm_compat_runner::CanvasText
                }) as Box<dyn Fn() -> pm_compat_runner::CanvasText + Send>
            }),
            wake: Arc::new(move || {
                let _ = wake_tx.try_send(());
            }),
        });
        rasters.connect(host.raster_inbox());
        self.extensions.doc_id = doc_id;
        self.extensions.overrides = Some(rasters);
        self.extensions.host = Some(host);
        self.sync_viewport();
        cx.spawn(async move |this, cx| {
            while wake_rx.recv().await.is_ok() {
                if this.update(cx, |v, cx| v.drain_extensions(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            while let Ok(request) = clock_rx.recv().await {
                let moved = this.update(cx, |v, cx| {
                    match request {
                        TransportRequest::Seek(time) => v.seek(time),
                        TransportRequest::Play | TransportRequest::Pause => {
                            if let Some(viewer) = &v.viewer {
                                let _ = viewer.command(if request == TransportRequest::Play {
                                    TransportCommand::Play {
                                        rate: PlaybackRate::NORMAL,
                                    }
                                } else {
                                    TransportCommand::Pause
                                });
                            }
                        }
                    }
                    cx.notify();
                });
                if moved.is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    /// Everything the host published since the last wake. Runs on main; never blocks.
    pub fn drain_extensions(&mut self, cx: &mut Context<Self>) {
        let Some(host) = &self.extensions.host else {
            return;
        };
        let revision = host.revision();
        // A rebuilt realm is a new document: render again so open panels
        // replace the page the old one drew into.
        if self
            .extensions
            .pages
            .get("")
            .is_some_and(|page| host.view() != 0 && page.view != host.view())
        {
            cx.notify();
        }
        for toast in host.take_toasts() {
            self.message = toast;
            cx.notify();
        }
        for request in host.take_requests() {
            self.extension_request(request, cx);
        }
        // The trusted realm's document gets its page as soon as the realm
        // starts, not only when a panel shows: hidden elements extensions
        // measure (a caret, a label) need real layout (plan 3.5.2).
        let trusted_page = self.extensions.host.as_ref().is_some_and(|h| h.view() != 0)
            && !self.extensions.pages.contains_key("");
        if trusted_page {
            self.ensure_page("", cx);
        }
        self.sync_export_pages(cx);
        if revision != self.extensions.seen {
            self.extensions.seen = revision;
            self.mount_surfaces(cx);
            if let Some(contributions) = self
                .extensions
                .host
                .as_ref()
                .and_then(|h| h.contributions())
            {
                self.apply_contributions(&contributions, cx);
            }
        }
        self.present_page();
    }
    /// The realm's questions for AppKit. Each answer returns through the inbox.
    fn extension_request(&mut self, request: Value, cx: &mut Context<Self>) {
        let token = request["token"].as_u64().unwrap_or_default();
        match request["kind"].as_str() {
            Some("panel") => {
                let id = request["id"].as_str().unwrap_or_default();
                let verb = if request["open"] == true {
                    "show"
                } else {
                    "close"
                };
                self.host
                    .borrow()
                    .send_command(&format!("panel:{verb}:{id}"), vec![]);
            }
            Some("confirm") => {
                let title = request["title"].as_str().unwrap_or_default();
                let body = request["body"].as_str().unwrap_or_default();
                let shell = self.host.borrow();
                use crate::alert::{Alert, Button};
                let answer = Alert::new(
                    title,
                    body,
                    vec![
                        Button::new("OK", true).default(),
                        Button::new("Cancel", false).cancel(),
                    ],
                )
                .sheet(&shell.window);
                drop(shell);
                cx.spawn(async move |this, cx| {
                    let confirmed = answer.recv().await == Ok(Some(true));
                    let _ = this.update(cx, |v, _| {
                        if let Some(host) = &v.extensions.host {
                            host.answer(token, confirmed.into());
                        }
                    });
                })
                .detach();
            }
            Some("unresponsive") => {
                let Some(execution) = request["execution"].as_u64() else {
                    return;
                };
                let restart = "You can wait for it to finish, or stop it and restart extensions. Open extension panels lose what they haven’t saved.";
                let (title, body) = match request["name"].as_str() {
                    Some(name) if request["disables"] == true => (
                        format!("“{name}” isn’t responding"),
                        format!(
                            "You can wait for it to finish, or stop it. A stopped extension stays off until you turn {name} back on in Extensions."
                        ),
                    ),
                    Some(name) => (format!("“{name}” isn’t responding"), restart.to_owned()),
                    None => (
                        "An extension isn’t responding".to_owned(),
                        restart.to_owned(),
                    ),
                };
                let shell = self.host.borrow();
                use crate::alert::{Alert, Button};
                let answer = Alert::new(
                    title,
                    body,
                    vec![
                        Button::new("Wait", false).default(),
                        Button::new("Stop", true),
                    ],
                )
                .sheet(&shell.window);
                drop(shell);
                self.extensions.unresponsive = Some(execution);
                cx.spawn(async move |this, cx| {
                    let stop = answer.recv().await == Ok(Some(true));
                    let _ = this.update(cx, |v, _| {
                        if v.extensions.unresponsive == Some(execution) {
                            v.extensions.unresponsive = None;
                        }
                        if let Some(host) = &v.extensions.host {
                            host.answer_unresponsive(execution, stop);
                        }
                    });
                })
                .detach();
            }
            Some("import") => match request["file"].as_str().and_then(|t| t.parse::<u64>().ok()) {
                Some(file) => {
                    let definition = request["layerDefinition"].as_str().map(str::to_owned);
                    self.import_for_extension(token, file, definition, cx)
                }
                None => {
                    if let Some(host) = &self.extensions.host {
                        host.answer(
                            token,
                            serde_json::json!({"error":"assets.import requires a File"}),
                        );
                    }
                }
            },
            // The `tool` service's setTool: only the shell's tool commands.
            Some("command") => {
                if let Some(name) = request["name"].as_str().filter(|n| {
                    n.starts_with("tool")
                        && n.len() > 4
                        && n.chars().all(|c| c.is_ascii_alphabetic())
                }) {
                    // A shape tool's shape rides along as the Shape tool's variant
                    // (v1's rect, rounded, ellipse, polygon, star).
                    let args = request["shape"]
                        .as_str()
                        .map(|shape| vec![serde_json::json!(shape)])
                        .unwrap_or_default();
                    self.host.borrow().send_command(name, args);
                }
            }
            // The `viewer` service's writable showControls: the viewer's layer controls.
            Some("viewerControls") => {
                if let Some(gizmos) = self.builtins.gizmos.clone() {
                    let shown = request["shown"] == true;
                    gizmos.update(cx, |g, cx| {
                        if g.controls() != shown {
                            g.toggle_controls(cx)
                        }
                    });
                }
            }
            // The `viewer` service's setZoom: the viewer leaves fit at that zoom
            // (as far as the display can show it) and reports where it landed.
            Some("viewerZoom") => {
                use pm_panel_viewerchrome::{ZOOM_MAX, ZOOM_MIN};
                if let Some(zoom) = request["zoom"].as_f64().filter(|z| z.is_finite()) {
                    let pan = match request["pan"].as_array().map(Vec::as_slice) {
                        Some([x, y]) => [x.as_f64().unwrap_or(0.), y.as_f64().unwrap_or(0.)],
                        _ => self.view.pan,
                    };
                    let top = ZOOM_MAX.min(self.zoom_limit()).max(ZOOM_MIN);
                    self.view.place(Some(zoom.clamp(ZOOM_MIN, top)), pan);
                    self.write_view(None, cx);
                }
            }
            // An extension made the viewer's stage: show the page over the viewer.
            Some("viewerStage") => {
                self.extensions.stage_used = true;
                self.extensions.stage_sent = None;
                self.sync_viewer_stage();
                cx.notify();
            }
            // The `timeline` service's writable view and its actions.
            Some("timeline") => {
                let Some(timeline) = self.builtins.timeline.clone() else {
                    return;
                };
                let time = self.time();
                timeline.update(cx, |t, cx| t.service_request(&request, time, cx));
            }
            Some("fonts") => self.ensure_page_fonts(&request["faces"], cx),
            Some("download") => {
                let url = request["url"].as_str().unwrap_or_default().to_owned();
                self.download_for_extension(token, url, cx);
            }
            Some("pick") => {
                let accept = request["accept"].as_str().unwrap_or_default().to_owned();
                self.pick_for_extension(token, &accept, request["multiple"] == true, cx);
            }
            Some("openExternal") => {
                let url = request["url"].as_str().unwrap_or_default();
                let opened = external_link(url) && crate::platform::open_url(url);
                if let Some(host) = &self.extensions.host {
                    host.answer(token, opened.into());
                }
            }
            Some("store") => {
                let entries = store_entries(request["items"].as_array().unwrap_or(&vec![]));
                let shell = self.host.borrow();
                let size = shell.gpui.bounds().size;
                shell.context_menu(&entries, size.width as f32 / 2., size.height as f32 / 2.);
            }
            Some("menu") => {
                let items = menu_entries(request["items"].as_array().unwrap_or(&vec![]), token);
                let open = self
                    .extensions
                    .menu
                    .take_if(|menu| Some(menu.key) == request["key"].as_u64());
                let shell = self.host.borrow();
                match open {
                    Some(OpenMenu {
                        mut entries, at, ..
                    }) => {
                        if !items.is_empty() {
                            entries.push(MenuEntry::separator());
                            entries.extend(items);
                        }
                        shell.context_menu(&entries, at.0, at.1);
                    }
                    None if request["key"].is_null() => shell.context_menu(
                        &items,
                        request["x"].as_f64().unwrap_or_default() as f32,
                        request["y"].as_f64().unwrap_or_default() as f32,
                    ),
                    None => (),
                }
            }
            _ => (),
        }
    }
    /// Extensions add to a tab's menu asynchronously: v4 realms answer first
    /// (100 ms at most, off the main thread), then v1–3 within the deadline.
    pub fn extension_panel_menu(
        &mut self,
        id: &str,
        entries: Vec<MenuEntry>,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) -> Option<Vec<MenuEntry>> {
        self.extension_menu(
            vec![("panel:context", json!({"panelId": id}))],
            entries,
            at,
            cx,
        )
    }
    /// A built-in context menu with what v4 and v1–3 extensions add for each
    /// of `locations` (v1–3's `MenuLocation` and its context, gathered in
    /// order). `None`: it opens once they answer.
    pub fn extension_menu(
        &mut self,
        locations: Vec<(&'static str, Value)>,
        entries: Vec<MenuEntry>,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) -> Option<Vec<MenuEntry>> {
        let compat = self.extensions.host.as_ref().is_some_and(|h| h.view() != 0);
        let runtime = self.builtins.runtime.clone().filter(|r| r.has_extensions());
        // With no realm nothing can add to it, so it opens right away.
        if !compat && runtime.is_none() {
            return Some(entries);
        }
        cx.spawn(async move |this, cx| {
            let (extra, locations) = match runtime {
                Some(runtime) => {
                    cx.background_executor()
                        .spawn(async move {
                            let extra = locations
                                .iter()
                                .flat_map(|(location, ctx)| v4_menu_items(&runtime, location, ctx))
                                .collect::<Vec<_>>();
                            (extra, locations)
                        })
                        .await
                }
                None => (vec![], locations),
            };
            let _ = this.update(cx, |v, cx| {
                let mut entries = entries;
                if !extra.is_empty() {
                    entries.push(MenuEntry::separator());
                    entries.extend(extra);
                }
                v.open_compat_menu(locations, entries, at, cx);
            });
        })
        .detach();
        None
    }
    /// True when some extension can add to a context menu.
    pub fn extensions_listen(&self) -> bool {
        self.extensions.host.as_ref().is_some_and(|h| h.view() != 0)
            || self
                .builtins
                .runtime
                .as_ref()
                .is_some_and(|r| r.has_extensions())
    }
    /// A built-in menu given as rows (`{id,label,disabled?,checked?}` or
    /// `null` for a separator) with extension items after them. A built-in
    /// row runs `choose` with its index, as the plain native menu would.
    pub fn extension_rows_menu(
        &mut self,
        locations: Vec<(&'static str, Value)>,
        rows: &[Value],
        at: (f32, f32),
        choose: crate::timeline_surface::Choose,
        cx: &mut Context<Self>,
    ) {
        self.extensions.next_menu += 1;
        let token = self.extensions.next_menu;
        let entries = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                if row.is_null() {
                    return MenuEntry::separator();
                }
                let mut entry = MenuEntry::item(
                    row["label"].as_str().unwrap_or_default(),
                    format!("{CHOICE}{token}:{i}"),
                )
                .checked(row["checked"] == true);
                entry.enabled = row["disabled"] != true;
                entry
            })
            .collect();
        self.extensions.choice = Some((token, choose));
        if let Some(entries) = self.extension_menu(locations, entries, at, cx) {
            self.host.borrow().context_menu(&entries, at.0, at.1);
        }
    }
    /// v1–3 realms add to the menu, which opens when they answer or the
    /// deadline passes.
    fn open_compat_menu(
        &mut self,
        locations: Vec<(&'static str, Value)>,
        entries: Vec<MenuEntry>,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.extensions.host.as_ref().filter(|h| h.view() != 0) else {
            self.host.borrow().context_menu(&entries, at.0, at.1);
            return;
        };
        self.extensions.next_menu += 1;
        let key = self.extensions.next_menu;
        let pairs = locations
            .into_iter()
            .map(|(location, ctx)| json!([location, ctx]))
            .collect();
        host.gather_menus(pairs, key);
        self.extensions.menu = Some(OpenMenu { key, entries, at });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MENU_DEADLINE).await;
            let _ = this.update(cx, |v, _| {
                if let Some(menu) = v.extensions.menu.take_if(|menu| menu.key == key) {
                    v.host
                        .borrow()
                        .context_menu(&menu.entries, menu.at.0, menu.at.1);
                }
            });
        })
        .detach();
    }
    /// Each v4 extension gets its own realm and draws through the v4 panel path.
    /// One that is disabled, removed or rebuilt loses that realm here. A fork
    /// (`replaces` a built-in) draws that built-in's panel instead, and the
    /// built-in comes back when the fork goes.
    fn mount_surfaces(&mut self, cx: &mut Context<Self>) {
        let (Some(host), Some(runtime)) = (&self.extensions.host, self.builtins.runtime.clone())
        else {
            return;
        };
        let wanted = host.surfaces();
        let ready = host.is_ready();
        let source = |surface: &Value| surface["graph"]["hash"].as_str().map(str::to_owned);
        let still = |id: &str, hash: &str| {
            wanted
                .iter()
                .any(|s| s["id"] == id && source(s).as_deref() == Some(hash))
        };
        let gone: Vec<String> = self
            .extensions
            .surfaces
            .iter()
            .filter(|(id, (_, hash))| !still(id, hash))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &gone {
            let Some(rebuilt) = wanted.iter().find(|s| s["id"] == id.as_str()) else {
                self.builtins.forget_surface(id);
                runtime.unmount_extension(id);
                self.extensions.surfaces.remove(id);
                let shell = self.host.borrow();
                shell.send_command(&format!("panel:close:{id}"), vec![]);
                continue;
            };
            // A new version: the old one hands over `api.hot.data`, then goes.
            let title = rebuilt["manifest"]["name"]
                .as_str()
                .unwrap_or(id)
                .to_owned();
            let hash = source(rebuilt).unwrap_or_default();
            self.extensions.surfaces.insert(id.clone(), (title, hash));
            self.builtins.keep_surface(id);
            let (runtime, extension, graph) =
                (runtime.clone(), id.clone(), rebuilt["graph"].clone());
            let ticket = cx.background_executor().spawn(async move {
                let hot = runtime.hot(&extension).ok().and_then(|ticket| {
                    let deadline = std::time::Instant::now() + HOT_HANDOFF;
                    crate::ticket::answer(ticket, deadline)?
                        .ok()
                        .map(|e| e.value)
                });
                runtime.unmount_extension(&extension);
                runtime.mount_extension(&extension, &graph, hot.as_deref())
            });
            self.await_panel(id.clone(), id.clone(), ticket, cx);
        }
        let unforked: Vec<(String, String)> = self
            .extensions
            .forks
            .iter()
            .filter(|(id, (_, hash))| !still(id, hash))
            .map(|(id, (panel, _))| (id.clone(), panel.clone()))
            .collect();
        for (id, panel) in unforked {
            self.extensions.forks.remove(&id);
            // A rebuilt fork takes its panel straight back below.
            let refork = wanted.iter().any(|s| {
                s["id"] == id.as_str()
                    && crate::extension_authoring::replaced_panel(&s["manifest"])
                        == Some(panel.as_str())
            });
            if !refork {
                self.builtins.keep_surface(&panel);
                let (runtime, built_in) = (runtime.clone(), panel.clone());
                let ticket = cx
                    .background_executor()
                    .spawn(async move { runtime.restore(&built_in) });
                self.await_panel(panel, id, ticket, cx);
            }
        }
        for surface in wanted {
            let Some(id) = surface["id"].as_str().map(str::to_owned) else {
                continue;
            };
            if self.extensions.surfaces.contains_key(&id) || self.extensions.forks.contains_key(&id)
            {
                continue;
            }
            let hash = source(&surface).unwrap_or_default();
            if let Some(panel) = crate::extension_authoring::replaced_panel(&surface["manifest"]) {
                if let Some((other, _)) =
                    self.extensions.forks.iter().find(|(_, (p, _))| p == panel)
                {
                    self.message =
                        format!("{id} and {other} both replace {panel}; {other} is kept.");
                    continue;
                }
                self.extensions
                    .forks
                    .insert(id.clone(), (panel.into(), hash));
                self.builtins.keep_surface(panel);
                let (runtime, fork, graph) =
                    (runtime.clone(), id.clone(), surface["graph"].clone());
                let ticket = cx
                    .background_executor()
                    .spawn(async move { runtime.replace(panel, &fork, &graph) });
                self.await_panel(panel.into(), id, ticket, cx);
                continue;
            }
            let title = surface["manifest"]["name"]
                .as_str()
                .unwrap_or(&id)
                .to_owned();
            self.extensions.surfaces.insert(id.clone(), (title, hash));
            let ticket = runtime.mount_extension(&id, &surface["graph"], None);
            self.await_panel(id.clone(), id.clone(), Task::ready(ticket), cx);
            // A newly installed panel shows itself; one already in the
            // workspace keeps its place, and the stack keeps its showing tab.
            if self.extensions.mounted_once.insert(id.clone())
                && self.extensions.booted
                && !self.extensions.open_panels.contains(&id)
            {
                self.host
                    .borrow()
                    .send_command(&format!("panel:show:{id}"), vec![]);
            }
        }
        self.extensions.booted |= ready;
    }
    /// The panel shows once its first commit is accepted; a failure says whose.
    fn await_panel(
        &mut self,
        panel: String,
        source: String,
        ticket: Task<pm_runtime_v8::Result<pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let mounted = match ticket.await {
                Ok(ticket) => ticket.await.map(|e| e.generation.number),
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |v, cx| {
                match mounted {
                    Ok(generation) => v.surface_ready(panel, generation),
                    Err(error) => {
                        v.builtins.forget_surface(&panel);
                        v.message = format!("{source} could not start: {error}");
                    }
                }
                v.drain_builtins(cx);
                cx.notify();
            });
        })
        .detach();
    }
    fn apply_contributions(&mut self, value: &Value, cx: &mut Context<Self>) {
        let panels: Vec<(String, String)> = value["panels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|panel| {
                let id = panel["id"].as_str()?.to_owned();
                let title = panel["title"].as_str().unwrap_or(&id).to_owned();
                Some((id, title))
            })
            .collect();
        self.extensions.stores = value["panels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|panel| {
                let store = panel["store"].as_str()?.to_owned();
                let view = panel["view"].as_u64().unwrap_or_default();
                Some((panel["id"].as_str()?.to_owned(), (store, view)))
            })
            .collect();
        *self.extensions.owners.borrow_mut() = self
            .extensions
            .stores
            .iter()
            .map(|(panel, (store, _))| (panel.clone(), store.clone()))
            .collect();
        // A Store extension that stopped takes its document with it.
        let running: BTreeSet<&String> = self.extensions.stores.values().map(|(s, _)| s).collect();
        let stopped: Vec<String> = self
            .extensions
            .pages
            .keys()
            .filter(|key| {
                !key.is_empty() && !key.starts_with(EXPORT_PAGE) && !running.contains(key)
            })
            .cloned()
            .collect();
        for key in stopped {
            self.extensions.pages.remove(&key);
            self.extensions.views.borrow_mut().remove(&key);
        }
        let known: BTreeSet<String> = panels.iter().map(|(id, _)| id.clone()).collect();
        // As in the shipped app: the workspace keeps a retired panel's place, and
        // it comes back if something registers the id again.
        let shell = self.host.borrow();
        for id in self.extensions.known_panels.difference(&known) {
            if self.extensions.open_panels.contains(id) || self.extensions.panels.contains_key(id) {
                self.extensions.suspended.insert(id.clone());
                shell.send_command(&format!("panel:park:{id}"), vec![]);
            }
        }
        for id in known.intersection(&self.extensions.suspended.clone()) {
            self.extensions.suspended.remove(id);
            shell.send_command(&format!("panel:show:{id}"), vec![]);
        }
        drop(shell);
        self.extensions.known_panels = known;
        let surfaces = self
            .extensions
            .surfaces
            .iter()
            .map(|(id, (title, _))| (id.clone(), title.clone()));
        let contributed: Vec<_> = panels.iter().cloned().chain(surfaces).collect();
        self.extensions.contributed = contributed;
        // New installs need a pinned Store key; until one ships there is no entry.
        let store = pm_compat_runner::store_install::available().then(|| Choice {
            label: "Install Extension from Store…".into(),
            command: STORE.into(),
        });
        let undo = Choice {
            label: "Undo Extension Change".into(),
            command: UNDO.into(),
        };
        let choices: Vec<Choice> = value["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|command| {
                let id = command["id"].as_str()?;
                Some(Choice {
                    label: command["label"]
                        .as_str()
                        .or(command["title"].as_str())
                        .unwrap_or(id)
                        .to_owned(),
                    command: format!("{COMMAND}{id}"),
                })
            })
            .chain(store)
            .chain([undo])
            .collect();
        self.extensions.commands = choices
            .iter()
            .filter_map(|c| c.command.strip_prefix(COMMAND).map(str::to_owned))
            .collect();
        let bindings: Vec<Binding> = value["keybindings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|binding| {
                let command = binding["command"].as_str()?;
                Some(Binding {
                    id: format!("{OWNER}{}", binding["id"].as_str()?),
                    chord: binding["chord"].as_str()?.to_owned(),
                    command: format!("{COMMAND}{command}"),
                    args: binding["args"].as_array().cloned().unwrap_or_default(),
                    priority: binding["priority"].as_i64().unwrap_or(0) as i32 + 300,
                    in_fields: binding["inFields"] == true,
                    repeat: binding["repeat"] == true,
                    loose: binding["looseModifiers"] == true,
                })
            })
            .collect();
        self.extensions.compat_choices = choices;
        let mut shell = self.host.borrow_mut();
        shell.contributed_choices = self.extensions.palette_choices();
        shell.set_contributed_bindings(OWNER, bindings);
        drop(shell);
        let definitions: Vec<_> = value["definitions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|definition| serde_json::from_value(definition.clone()).ok())
            .collect();
        if definitions != self.extensions.resources.definitions {
            self.extensions.resources = Arc::new(ResourceSnapshot {
                generation: self.extensions.resources.generation + 1,
                definitions,
                ..Default::default()
            });
            if let Some(doc_id) = self.extensions.doc_id {
                crate::mcp_services::set_resources(doc_id, Some(self.extensions.resources.clone()));
            }
            self.apply_extension_resources();
        }
        self.apply_v1_theme(&value["nativeTheme"], cx);
        self.update_builtins(cx);
        cx.notify();
    }
    /// The active v1–3 theme, translated in the realm (plan 3.6.5), restyles
    /// every window's chrome and native panels when it changes, in both
    /// appearances. Choosing a v4 theme afterwards replaces it until this one
    /// changes; another window publishing the same theme changes nothing.
    fn apply_v1_theme(&mut self, theme: &Value, cx: &mut Context<Self>) {
        if theme.is_null() {
            return;
        }
        self.extensions.v1_timeline = theme["timeline"].clone();
        let key = theme.to_string();
        if V1_THEME.with(|t| t.borrow().as_ref().is_some_and(|(k, _)| *k == key)) {
            return;
        }
        if let Some(error) = theme["error"].as_str() {
            let name = theme["id"].as_str().unwrap_or("theme");
            self.errors.borrow_mut().tell(
                "Theme",
                &format!("The theme “{name}” couldn’t be applied: {error}"),
            );
            V1_THEME.with(|t| *t.borrow_mut() = Some((key, Default::default())));
            return;
        }
        let light = if theme["light"].is_object() {
            &theme["light"]
        } else {
            &theme["dark"]
        };
        match theme_patch(&theme["dark"]).and_then(|dark| Ok((dark, theme_patch(light)?))) {
            Ok(patches) => {
                V1_THEME.with(|t| *t.borrow_mut() = Some((key, patches.clone())));
                ACTIVE_THEME.with(|a| *a.borrow_mut() = None);
                apply_theme(patches, cx);
            }
            Err(message) => self.errors.borrow_mut().tell("Theme", &message),
        }
    }
    /// Every live viewer renders the same contributed definitions.
    pub fn apply_extension_resources(&mut self) {
        if let Some(viewer) = &self.viewer
            && let Err(error) = viewer.resources(self.extensions.resources.clone())
        {
            self.message = error.to_string();
        }
    }
    pub fn extension_command(&mut self, cmd: &ShellCommand, cx: &mut Context<Self>) -> bool {
        if cmd.name == UNDO {
            self.undo_extension_change(cx);
            return true;
        }
        if cmd.name == STORE {
            if let Some(host) = &self.extensions.host {
                host.browse_store();
            }
            return true;
        }
        if let Some(install) = cmd.name.strip_prefix(INSTALL) {
            if let (Some(host), Some((release, repo))) =
                (&self.extensions.host, install.split_once(':'))
            {
                host.install_store(release, repo);
            }
            return true;
        }
        if let Some(choice) = cmd.name.strip_prefix(MENU) {
            if let (Some(host), Some((token, run))) =
                (&self.extensions.host, choice.split_once(':'))
                && let (Ok(token), Ok(run)) = (token.parse(), run.parse::<u64>())
            {
                host.answer(token, run.into());
            }
            return true;
        }
        if let Some(target) = cmd.name.strip_prefix(CHOICE) {
            if let Some((token, index)) = target.split_once(':')
                && let (Ok(token), Ok(index)) = (token.parse::<u64>(), index.parse::<usize>())
                && let Some((_, choose)) = self
                    .extensions
                    .choice
                    .take_if(|(pending, _)| *pending == token)
            {
                choose(Some(index), cx);
            }
            return true;
        }
        if let Some(target) = cmd.name.strip_prefix(V4_MENU) {
            if let (Some(runtime), Some((panel, key))) =
                (self.builtins.runtime.clone(), target.split_once(':'))
            {
                let (panel, key) = (panel.to_owned(), key.to_owned());
                cx.spawn(async move |this, cx| {
                    let outcome = cx
                        .background_executor()
                        .spawn(async move {
                            runtime
                                .run_menu(&panel, &key)
                                .map_err(|e| format!("{e:?}"))
                                .and_then(|ticket| {
                                    wait_ticket(
                                        ticket,
                                        std::time::Instant::now()
                                            + std::time::Duration::from_secs(30),
                                    )
                                })
                        })
                        .await;
                    if let Err(message) = outcome {
                        let _ = this
                            .update(cx, |v, _| v.errors.borrow_mut().tell("Extension", &message));
                    }
                })
                .detach();
            }
            return true;
        }
        if let Some(target) = cmd.name.strip_prefix(V4_THEME) {
            let chosen = (target != "default").then(|| target.to_owned());
            let tokens = chosen.as_deref().and_then(|key| self.v4_theme(key));
            match (chosen, tokens) {
                (None, _) => {
                    ACTIVE_THEME.with(|a| *a.borrow_mut() = None);
                    apply_theme(base_look(), cx);
                }
                (Some(key), Some(tokens)) => match theme_patch(&tokens) {
                    Ok(patch) => {
                        ACTIVE_THEME.with(|a| *a.borrow_mut() = Some(key));
                        apply_theme((patch.clone(), patch), cx);
                    }
                    Err(message) => self.errors.borrow_mut().tell("Theme", &message),
                },
                (Some(_), None) => {}
            }
            return true;
        }
        if let Some(target) = cmd.name.strip_prefix(V4_COMMAND) {
            if let (Some(runtime), Some((panel, id))) =
                (self.builtins.runtime.clone(), target.split_once(':'))
            {
                let (panel, id, args) = (panel.to_owned(), id.to_owned(), cmd.args.clone());
                cx.spawn(async move |this, cx| {
                    let ticket = cx
                        .background_executor()
                        .spawn(async move { runtime.run_command(&panel, &id, &args) })
                        .await;
                    let result = match ticket {
                        Ok(ticket) => ticket.await.map(drop),
                        Err(error) => Err(error),
                    };
                    if let Err(error) = result {
                        let _ = this.update(cx, |v, cx| {
                            v.message = format!("The command failed: {error}");
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            return true;
        }
        let Some(id) = cmd.name.strip_prefix(COMMAND) else {
            return false;
        };
        if let Some(host) = &self.extensions.host {
            host.run_command(id, Value::Array(cmd.args.clone()));
        }
        true
    }
    fn v4_theme(&self, key: &str) -> Option<Value> {
        self.extensions
            .v4_themes
            .iter()
            .find(|t| {
                format!(
                    "{}:{}",
                    t["panel"].as_str().unwrap_or_default(),
                    t["id"].as_str().unwrap_or_default()
                ) == key
            })
            .map(|t| t["tokens"].clone())
    }
    /// Themes v4 extensions offer go into the palette ("Theme: <title>"). The
    /// chosen one follows its extension: a reload restyles, a removal resets.
    pub fn sync_v4_themes(&mut self, runtime: &pm_builtins::Builtins, cx: &mut Context<Self>) {
        let Some((revision, list)) = runtime.themes_since(self.extensions.v4_themes_seen) else {
            return;
        };
        self.extensions.v4_themes_seen = revision;
        self.extensions.v4_themes = list;
        let mut choices: Vec<Choice> = self
            .extensions
            .v4_themes
            .iter()
            .map(|t| Choice {
                label: format!("Theme: {}", t["title"].as_str().unwrap_or_default()),
                command: format!(
                    "{V4_THEME}{}:{}",
                    t["panel"].as_str().unwrap_or_default(),
                    t["id"].as_str().unwrap_or_default()
                ),
            })
            .collect();
        if !choices.is_empty() {
            choices.push(Choice {
                label: "Theme: Default".into(),
                command: format!("{V4_THEME}default"),
            });
        }
        self.extensions.v4_theme_choices = choices;
        self.host.borrow_mut().contributed_choices = self.extensions.palette_choices();
        // The latest `themes.activate` in the window shows its theme, as v1–3's
        // `theme.activate`; a later palette choice still wins. Each request
        // acts once, so activating the same theme again shows it again.
        if let Some((number, panel, id)) = runtime.theme_request()
            && self.extensions.v4_theme_requested != Some(number)
        {
            self.extensions.v4_theme_requested = Some(number);
            let prefix = format!("{panel}:");
            match id {
                Some(id) => ACTIVE_THEME.with(|a| *a.borrow_mut() = Some(format!("{prefix}{id}"))),
                // `activate(null)` puts back what this extension's theme replaced.
                None if ACTIVE_THEME
                    .with(|a| a.borrow().as_ref().is_some_and(|k| k.starts_with(&prefix))) =>
                {
                    ACTIVE_THEME.with(|a| *a.borrow_mut() = None);
                    apply_theme(base_look(), cx);
                }
                None => {}
            }
        }
        if let Some(active) = ACTIVE_THEME.with(|a| a.borrow().clone()) {
            match self.v4_theme(&active).map(|tokens| theme_patch(&tokens)) {
                Some(Ok(patch)) => apply_theme((patch.clone(), patch), cx),
                Some(Err(message)) => self.errors.borrow_mut().tell("Theme", &message),
                None => {
                    ACTIVE_THEME.with(|a| *a.borrow_mut() = None);
                    apply_theme(base_look(), cx);
                }
            }
        }
    }
    /// Commands v4 extensions registered since the last look go into the
    /// palette and, with a `key`, the keymap.
    pub fn sync_v4_commands(&mut self, runtime: &pm_builtins::Builtins) {
        let Some((revision, list)) = runtime.commands_since(self.extensions.v4_seen) else {
            return;
        };
        self.extensions.v4_seen = revision;
        let target = |c: &Value| {
            format!(
                "{V4_COMMAND}{}:{}",
                c["panel"].as_str().unwrap_or_default(),
                c["id"].as_str().unwrap_or_default()
            )
        };
        self.extensions.v4_choices = list
            .iter()
            .map(|c| Choice {
                label: c["title"].as_str().unwrap_or_default().to_owned(),
                command: target(c),
            })
            .collect();
        let bindings: Vec<Binding> = list
            .iter()
            .filter_map(|c| {
                Some(Binding {
                    id: format!("{V4_OWNER}{}", target(c)),
                    chord: c["key"].as_str()?.to_owned(),
                    command: target(c),
                    args: vec![],
                    priority: 300,
                    in_fields: false,
                    repeat: false,
                    loose: false,
                })
            })
            .collect();
        let mut shell = self.host.borrow_mut();
        shell.contributed_choices = self.extensions.palette_choices();
        shell.set_contributed_bindings(V4_OWNER, bindings);
    }
    /// Puts back what the newest agent extension change replaced, for this
    /// project or an app-wide run. Files move on a worker; windows reload.
    fn undo_extension_change(&mut self, cx: &mut Context<Self>) {
        let extensions = crate::extension_authoring::AppExtensions::new(self.profile.clone());
        let project = self.session.as_ref().map(|s| s.project.clone());
        let task = cx.background_executor().spawn(async move {
            use pm_mcp::extensions::{change_sets, history_for, restore};
            let newest = [project.as_deref(), Some("powermove-global")]
                .into_iter()
                .flatten()
                .filter_map(|p| {
                    let set = change_sets(&history_for(&extensions, p))
                        .into_iter()
                        .next()?;
                    Some((p.to_owned(), set))
                })
                .max_by(|a, b| a.1["createdAt"].as_str().cmp(&b.1["createdAt"].as_str()));
            let Some((project, set)) = newest else {
                return "No extension change to undo.".to_owned();
            };
            match restore(
                &extensions,
                &project,
                set["id"].as_str().unwrap_or_default(),
            ) {
                Ok(record) => {
                    let names: Vec<&str> = record["changes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|c| c["id"].as_str())
                        .collect();
                    format!("Undid the change to {}.", names.join(", "))
                }
                Err(error) => error.message,
            }
        });
        cx.spawn(async move |this, cx| {
            let message = task.await;
            let _ = this.update(cx, |v, cx| {
                v.message = message;
                cx.notify();
            });
        })
        .detach();
    }
    /// A panel body is a hole in GPUI; the shared document shows through it.
    pub fn extension_panel(
        &mut self,
        id: &str,
        bounds: layout::Rect,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.extensions.owns_panel(id) {
            return None;
        }
        let t = self.host.borrow().tokens.clone();
        let body = layout::Rect {
            x: bounds.x + t.metrics.border,
            y: bounds.y + t.metrics.header,
            w: (bounds.w - t.metrics.border * 2.).max(0.),
            h: (bounds.h - t.metrics.header - t.metrics.border).max(0.),
        };
        self.extensions.hidden.remove(id);
        if self.extensions.panels.insert(id.to_owned(), body).is_none()
            && let Some(host) = &self.extensions.host
        {
            host.set_panel(id, true);
        }
        self.ensure_page(id, cx);
        self.place_panels();
        Some(
            div()
                .w_full()
                .h(px(body.h))
                .id(SharedString::from(format!("extension-panel-{id}")))
                .into_any_element(),
        )
    }
    /// The trusted realm's document, or a Store extension's own.
    fn ensure_page(&mut self, id: &str, cx: &mut Context<Self>) {
        let key = self
            .extensions
            .stores
            .get(id)
            .map_or(String::new(), |(store, _)| store.clone());
        let trusted = self.extensions.host.as_ref().map_or(0, |h| h.view());
        let view = self
            .extensions
            .stores
            .get(id)
            .map_or(trusted, |(_, view)| *view);
        if self
            .extensions
            .pages
            .get(&key)
            .is_some_and(|page| page.view == view)
        {
            return;
        }
        self.open_page(key, view, cx);
    }
    /// A realm's project text uses these faces (`PM.Fonts.ensure` too): the
    /// engine's own face for each goes to every page of the window.
    fn ensure_page_fonts(&mut self, faces: &Value, cx: &mut Context<Self>) {
        let wanted: Vec<(String, u16, bool)> = faces
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|face| {
                let family = face["family"].as_str()?.trim();
                let weight = face["weight"].as_f64().unwrap_or(400.).clamp(1., 1000.) as u16;
                (!family.is_empty() && family.len() <= 256)
                    .then(|| (family.to_owned(), weight, face["italic"] == true))
            })
            .filter(|key| self.extensions.face_keys.insert(key.clone()))
            .take(256)
            .collect();

        if wanted.is_empty() {
            return;
        }
        let keys: Vec<String> = wanted
            .iter()
            .map(|(family, weight, italic)| format!("{family}|{weight}|{}", u8::from(*italic)))
            .collect();
        let resolved = cx
            .background_executor()
            .spawn(async move { crate::text_host::page_faces(&wanted) });
        cx.spawn(async move |this, cx| {
            let faces = resolved.await;
            let _ = this.update(cx, |v, _| {
                for page in v.extensions.views.borrow().values() {
                    let _ = page.borrow_mut().add_fonts(&faces);
                }
                // A face the registry could not give is settled now: no page
                // will load it, and whoever waits for it stops waiting.
                let unresolved: Vec<Value> = keys
                    .iter()
                    .filter(|key| !faces.iter().any(|f| &f.key() == *key))
                    .map(|key| json!(key))
                    .collect();
                if let (false, Some(host)) = (unresolved.is_empty(), &v.extensions.host) {
                    for key in v.extensions.pages.keys() {
                        host.fonts_loaded(key, Value::Array(unresolved.clone()));
                    }
                }
                v.extensions.faces.extend(faces);
            });
        })
        .detach();
    }
    /// Each running export realm gets a page that is never shown: the layout
    /// reads its overrides make are real; it goes when the export ends.
    fn sync_export_pages(&mut self, cx: &mut Context<Self>) {
        let Some(host) = &self.extensions.host else {
            return;
        };
        let running: BTreeMap<String, u64> = host
            .export_pages()
            .into_iter()
            .map(|(job, view)| (format!("{EXPORT_PAGE}{job}"), view))
            .collect();
        let ended: Vec<String> = self
            .extensions
            .pages
            .keys()
            .filter(|key| key.starts_with(EXPORT_PAGE) && !running.contains_key(*key))
            .cloned()
            .collect();
        for key in ended {
            self.extensions.pages.remove(&key);
            if let Some(view) = self.extensions.views.borrow_mut().remove(&key) {
                view.borrow_mut().end_background_job();
            }
        }
        for (key, view) in running {
            if !self.extensions.pages.contains_key(&key) {
                self.open_page(key, view, cx);
            }
        }
    }
    fn open_page(&mut self, key: String, view: u64, cx: &mut Context<Self>) {
        // That run is over: its document goes, and its replacement starts empty.
        self.extensions.pages.remove(&key);
        self.extensions.views.borrow_mut().remove(&key);
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let (input, output) = std::sync::mpsc::channel::<Value>();
        let (wake_tx, wake_rx) = async_channel::bounded(1);
        let events = Rc::new(PageEvents {
            host: Arc::new(Mutex::new(Some(input))),
            events: RefCell::default(),
            wake: Rc::new(move || {
                let _ = wake_tx.try_send(());
            }),
        });
        let session = self
            .extensions
            .host
            .as_ref()
            .and_then(|h| h.store_session(&key));
        let tokens = self.host.borrow().tokens.clone();
        let bundle = pm_compat_runner::page_bundle(
            tokens.scheme == pm_design::Scheme::Dark,
            tokens.accessibility.increase_contrast,
            tokens.accessibility.reduce_transparency,
        );
        let created = match &session {
            Some(session) => MacDomHost::new_store_remote_dom(
                mtm,
                session,
                events.clone(),
                &mut self.extensions.data_stores,
                &bundle,
            ),
            None if key.is_empty() || key.starts_with(EXPORT_PAGE) => MacDomHost::new_remote_dom(
                mtm,
                Trust::Trusted,
                events.clone(),
                &mut self.extensions.data_stores,
                &bundle,
            ),
            None => return,
        };
        let mut host = match created {
            Ok(host) => host,
            Err(error) => {
                self.message = format!("Extension panels could not start: {error}");
                return;
            }
        };
        let spec = PanelSpec {
            id: PAGE_PANEL,
            label: "Extension panels".into(),
        };
        if host.create(spec).is_err() {
            return;
        }
        let _ = host.add_fonts(&self.extensions.faces);
        // An export's page keeps running for its job, the window hidden or not.
        if key.starts_with(EXPORT_PAGE) && host.begin_background_job(false).is_err() {
            eprintln!("an export page could not keep running in the background");
        }
        let shell = self.host.borrow();
        host.install_below(&shell.native_container(), &shell.gpui);
        drop(shell);
        let page = Rc::new(RefCell::new(host));
        self.extensions
            .views
            .borrow_mut()
            .insert(key.clone(), page.clone());
        let views = self.extensions.views.clone();
        let owners = self.extensions.owners.clone();
        let mut shell = self.host.borrow_mut();
        let bodies = shell.native_panels.clone();
        let stage = self.extensions.stage.clone();
        shell.native_present = Some(Rc::new(move |width, height| {
            let viewport = Rect::new(0., 0., width, height);
            let bodies = bodies.borrow();
            let owners = owners.borrow();
            let mut dirty = false;
            for (key, page) in views.borrow().iter() {
                let mut host = page.borrow_mut();
                let stage = stage.get().filter(|_| key.is_empty());
                let regions = bodies
                    .iter()
                    .filter(|(id, _)| owners.get(id).map_or("", String::as_str) == key)
                    .map(|(_, r)| r)
                    .chain(stage.as_ref())
                    .map(|r| Rect::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64))
                    .collect();
                let _ = host.set_regions(regions);
                dirty |= host.composition_dirty();
                let _ = host.attach(
                    PAGE_PANEL,
                    PanelSlot {
                        bounds: viewport,
                        clip: viewport,
                    },
                );
                let _ = host.present(viewport);
            }
            dirty
        }));
        // An export realm's page is never shown, so VoiceOver never meets it.
        shell.native_accessibility = self
            .extensions
            .views
            .borrow()
            .iter()
            .filter(|(key, _)| !key.starts_with(EXPORT_PAGE))
            .map(|(_, page)| page.borrow().accessibility_view())
            .collect();
        drop(shell);
        self.extensions.pages.insert(
            key.clone(),
            Page {
                host: page,
                events,
                ready: false,
                view,
            },
        );
        // Feedback reaches its realm on a worker, never inside the page's callback.
        let host = self.extensions.host.as_ref();
        let sink: Option<Box<dyn Fn(Value) + Send>> = if key.is_empty() {
            host.map(|h| Box::new(h.page_sink(view)) as _)
        } else if let Some(job) = key.strip_prefix(EXPORT_PAGE) {
            host.map(|h| Box::new(h.export_page_sink(job)) as _)
        } else {
            host.map(|h| Box::new(h.store_page_sink(&key, view)) as _)
        };
        std::thread::spawn(move || {
            while let Ok(message) = output.recv() {
                if let Some(sink) = &sink {
                    sink(message);
                }
            }
        });
        cx.spawn(async move |this, cx| {
            while wake_rx.recv().await.is_ok() {
                if this.update(cx, |v, _| v.present_page()).is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    /// Sends placement after layout: positions live in the realm's document.
    /// v1–3's `window.innerWidth` & co. follow the editor window's content.
    fn sync_viewport(&mut self) {
        let (size, scale) = {
            let shell = self.host.borrow();
            let view = &shell.gpui;
            let scale = view.window().map_or(2., |w| w.backingScaleFactor()) as f32;
            (view.frame().size, scale)
        };
        let viewport = (size.width as f32, size.height as f32, scale);
        if viewport.0 > 0.
            && self.extensions.viewport != Some(viewport)
            && let Some(host) = &self.extensions.host
        {
            host.set_viewport(viewport.0, viewport.1, viewport.2);
            self.extensions.viewport = Some(viewport);
        }
    }
    /// v1–3's viewer stage follows the viewer slot and its composition frame,
    /// in window points; the realm hears only when either moved.
    pub(crate) fn sync_viewer_stage(&mut self) {
        let Some(host) = &self.extensions.host else {
            return;
        };
        let slot = self.host.borrow().viewer_rect.get();
        let zoom = self.view.scale.get();
        let placed = slot.filter(|s| s.w > 0. && s.h > 0.).map(|s| {
            let (w, h) = (
                (self.dimensions[0] * zoom) as f32,
                (self.dimensions[1] * zoom) as f32,
            );
            let x = s.x + (s.w - w) / 2. + self.view.pan[0] as f32;
            let y = s.y + (s.h - h) / 2. + self.view.pan[1] as f32;
            ([s.x, s.y, s.w, s.h], [x, y, w, h])
        });
        let key = placed.map(|(a, b)| {
            let mut key = [0; 8];
            for (k, v) in key.iter_mut().zip(a.iter().chain(&b)) {
                *k = (v * 100.).round() as i64;
            }
            key
        });
        if self.extensions.stage_sent == Some(key) {
            return;
        }
        self.extensions.stage_sent = Some(key);
        let shown = slot.filter(|_| self.extensions.stage_used && placed.is_some());
        self.extensions.stage.set(shown);
        let (stage, frame) =
            placed.map_or((Value::Null, Value::Null), |(a, b)| (json!(a), json!(b)));
        host.shell_state(json!({ "stage": stage, "frame": frame }));
    }
    /// A new viewer frame on screen: v1–3's `frame:rendered` and overlay pass.
    pub(crate) fn tell_presented(&mut self, frame: Option<&pm_engine_service::FrameReport>) {
        let (Some(host), Some(frame)) = (&self.extensions.host, frame) else {
            return;
        };
        if self.extensions.presented == Some(frame.stamp.sequence) {
            return;
        }
        self.extensions.presented = Some(frame.stamp.sequence);
        let time = frame.time.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
        host.frame_presented(json!({
            "time": time,
            "version": frame.stamp.revision,
            "quality": frame.scale,
        }));
    }
    fn place_panels(&mut self) {
        self.sync_viewport();
        self.sync_viewer_stage();
        let Some(host) = &self.extensions.host else {
            return;
        };
        if self.extensions.placed == self.extensions.panels {
            return;
        }
        for (id, rect) in &self.extensions.panels {
            if !self.extensions.hidden.contains(id) && self.extensions.placed.get(id) != Some(rect)
            {
                host.place_panel(id, Some([rect.x, rect.y, rect.w, rect.h]));
            }
        }
        self.extensions.placed = self
            .extensions
            .panels
            .iter()
            .filter(|(id, _)| !self.extensions.hidden.contains(*id))
            .map(|(id, rect)| (id.clone(), *rect))
            .collect();
    }
    /// A tab behind another stays open in the realm but leaves the shared
    /// document's paint and hit testing; `extension_panel` brings it back.
    pub fn hide_extension_panels(&mut self, shown: &[&str]) {
        let Some(host) = &self.extensions.host else {
            return;
        };
        for id in self.extensions.panels.keys() {
            if !shown.contains(&id.as_str()) && self.extensions.hidden.insert(id.clone()) {
                self.extensions.placed.remove(id);
                host.place_panel(id, None);
            }
        }
    }
    /// The workspace closed a contributed panel: unmount it in the realm.
    pub fn sync_extension_panels(&mut self, open: &[&str]) {
        self.extensions.open_panels = open.iter().map(|id| (*id).to_owned()).collect();
        let closed: Vec<String> = self
            .extensions
            .panels
            .keys()
            .filter(|id| !open.contains(&id.as_str()))
            .cloned()
            .collect();
        for id in closed {
            self.close_extension_panel(&id);
        }
    }
    fn close_extension_panel(&mut self, id: &str) {
        self.extensions.placed.remove(id);
        self.extensions.hidden.remove(id);
        if self.extensions.panels.remove(id).is_some()
            && let Some(host) = &self.extensions.host
        {
            host.set_panel(id, false);
        }
    }
    fn present_page(&mut self) {
        let Some(host) = &self.extensions.host else {
            return;
        };
        for (key, page) in &mut self.extensions.pages {
            for event in page.events.events.borrow_mut().drain(..) {
                match event {
                    HostEvent::Ready { .. } => page.ready = true,
                    HostEvent::ProcessTerminated => page.ready = false,
                    HostEvent::Failure(code) => eprintln!("extension page: {code}"),
                    HostEvent::FontsLoaded(keys) => host.fonts_loaded(key, keys.into()),
                }
            }
            if !page.ready {
                continue;
            }
            let export = key.strip_prefix(EXPORT_PAGE);
            let mut batches = if key.is_empty() {
                host.take_batches(page.view)
            } else if let Some(job) = export {
                host.take_export_batches(job)
            } else {
                host.take_store_batches(key, page.view)
            };
            let mut web = page.host.borrow_mut();
            while let Some(batch) = batches.front() {
                if batch.present(&mut *web).is_err() {
                    break;
                }
                batches.pop_front();
            }
            if key.is_empty() {
                host.return_batches(page.view, batches);
                if page.view == host.view() {
                    for frame in host.take_canvases() {
                        let _ = web.present_canvas(PAGE_PANEL, &frame);
                    }
                }
            } else if let Some(job) = export {
                host.return_export_batches(job, batches);
            } else {
                host.return_store_batches(key, page.view, batches);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{external_link, menu_entries, store_entries};
    use serde_json::json;

    #[test]
    fn extension_menus_keep_their_choice_tokens_and_only_web_links_open() {
        let items = menu_entries(
            &[
                json!({"label":"Run","run":3,"on":true}),
                json!({"separator":true}),
                json!({"label":"Header","disabled":true}),
                json!({"label":"More","submenu":[{"label":"Nested","run":4}]}),
            ],
            9,
        );
        assert_eq!(items[0].command, "extension.menu:9:3");
        assert!(items[0].checked && items[0].enabled);
        assert!(items[1].label.is_empty());
        assert!(!items[2].enabled);
        assert_eq!(items[3].children[0].command, "extension.menu:9:4");
        assert!(external_link("https://example.com/a"));
        assert!(!external_link("file:///etc/passwd") && !external_link("powermove://x"));
        let store = store_entries(&[json!({"label":"First","release":"one","repo":"r"})]);
        assert_eq!(store[0].command, "extension.install:one:r");
    }
}

/// A v4 theme's tokens as the shell's patch: colours `"#rrggbb"`, the rest
/// numbers. Unknown tokens are refused by the shell when it applies the patch.
fn theme_patch(tokens: &Value) -> Result<pm_shell::theme::Patch, String> {
    let fields = tokens.as_object().ok_or("A theme needs a tokens object.")?;
    fields
        .iter()
        .map(|(name, value)| {
            let number = match value {
                Value::Number(n) => n.as_f64(),
                Value::String(hex) => hex
                    .strip_prefix('#')
                    .filter(|h| h.len() == 6)
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .map(f64::from),
                _ => None,
            };
            number
                .map(|n| (name.clone(), n))
                .ok_or_else(|| format!("Theme token {name} needs a number or a #rrggbb colour."))
        })
        .collect()
}
/// Restyles every window with a dark and a light patch (empty: the built-in
/// look), after the current update so each window's editor updates in turn.
fn apply_theme((dark, light): Patches, cx: &mut App) {
    cx.defer(move |cx| {
        for window in cx.windows() {
            let Some(editor) = window.downcast::<pm_shell::view::Editor>() else {
                continue;
            };
            let (dark, light) = (dark.clone(), light.clone());
            let _ = editor.update(cx, |e, _, cx| e.apply_themes(dark, light, cx));
        }
    });
}
/// What shows without a v4 theme: the active v1–3 theme, or the built-in look.
fn base_look() -> Patches {
    V1_THEME.with(|t| {
        t.borrow()
            .as_ref()
            .map(|(_, p)| p.clone())
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod theme_tests {
    use super::theme_patch;
    use serde_json::json;
    #[::core::prelude::v1::test]
    fn a_v4_theme_becomes_the_shell_patch() {
        let patch = theme_patch(&json!({"colors.accent":"#ff8800","metrics.gap":8})).unwrap();
        assert!(patch.contains(&("colors.accent".into(), f64::from(0xff8800u32))));
        assert!(patch.contains(&("metrics.gap".into(), 8.)));
        pm_shell::theme::set(patch).unwrap();
        pm_shell::theme::clear();
        assert!(theme_patch(&json!({"colors.accent":"orange"})).is_err());
        let unknown = theme_patch(&json!({"colors.nonsense":"#000000"})).unwrap();
        assert!(
            pm_shell::theme::set(unknown).is_err(),
            "unknown tokens are refused"
        );
    }
}

/// Waits for a realm evaluation on a worker until `deadline`.
fn wait_ticket(
    ticket: pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>,
    deadline: std::time::Instant,
) -> Result<String, String> {
    crate::ticket::answer(ticket, deadline)
        .ok_or_else(|| "The extension didn't answer in time.".to_owned())?
        .map(|e| e.value)
        .map_err(|e| format!("{e:?}"))
}
/// Every v4 extension's items for a menu, as native entries (off the main
/// thread). Realms gather in parallel, each giving its contributions 100 ms;
/// one 150 ms deadline covers them all.
fn v4_menu_items(runtime: &pm_builtins::Builtins, location: &str, ctx: &Value) -> Vec<MenuEntry> {
    let mut entries = vec![];
    let tickets = runtime.gather_menus(location, ctx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
    for (panel, ticket) in tickets {
        let Some(text) = ticket.ok().and_then(|t| wait_ticket(t, deadline).ok()) else {
            continue;
        };
        // The realm returns the items as JSON text.
        let items = serde_json::from_str::<Value>(&text)
            .ok()
            .map(|v| match v {
                Value::String(inner) => serde_json::from_str(&inner).unwrap_or(Value::Null),
                other => other,
            })
            .unwrap_or(Value::Null);
        entries.extend(v4_entries(&panel, &items));
    }
    entries
}
fn v4_entries(panel: &str, items: &Value) -> Vec<MenuEntry> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            if item == "-" {
                return Some(MenuEntry::separator());
            }
            if let Some(header) = item["header"].as_str() {
                let mut entry = MenuEntry::item(header, "");
                entry.enabled = false;
                return Some(entry);
            }
            let label = item["label"].as_str()?;
            if let Some(children) = item["submenu"].as_array() {
                return Some(MenuEntry::submenu(
                    label,
                    v4_entries(panel, &Value::Array(children.clone())),
                ));
            }
            let command = match (item["key"].as_str(), item["disabled"] == true) {
                (Some(key), false) => format!("{V4_MENU}{panel}:{key}"),
                _ => String::new(),
            };
            let mut entry = MenuEntry::item(label, command);
            if item["on"] == true {
                entry = entry.checked(true);
            }
            entry.enabled = !entry.command.is_empty();
            Some(entry)
        })
        .collect()
}
