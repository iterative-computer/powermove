//! The running app's v1–3 extensions. `ExtensionHost` owns V8 on its own thread;
//! this side only reads published copies, routes page messages and places the
//! one WebKit document below GPUI. Nothing here can wait on extension code.
use crate::session::Controller;
use gpui::{prelude::*, *};
use objc2::MainThreadMarker;
use pm_compat_host::{
    DomPanelHost, Error as PageError, HostEvent, MacDomHost, PanelSlot, PanelSpec, Rect,
    RequestContext, Result as PageResult, StoreDataStores, SyncHandler, Trust,
};
use pm_compat_runner::{
    PAGE_BUNDLE,
    host::{ExtensionHost, HostConfig},
    transport::{TransportOwner, TransportRequest},
};
use pm_engine_service::resources::ResourceSnapshot;
use pm_node::fonts::render::BrokerResolver;
use pm_protocol::{clock::TransportCommand, common::PlaybackRate};
use pm_shell::{Command as ShellCommand, MenuEntry, keyboard::Binding, layout, transient::Choice};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
};

const PAGE_PANEL: u64 = 1;
const COMMAND: &str = "extension.command:";
const MENU: &str = "extension.menu:";
const STORE: &str = "extension.store";
const INSTALL: &str = "extension.install:";
/// Longer than the realm's own deadline; a hung realm never holds a menu back.
const MENU_DEADLINE: std::time::Duration = std::time::Duration::from_millis(150);
const OWNER: &str = "extension:";
/// Keys this window's list of contributed panels in the shell.
static NEXT_WINDOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Page messages enter here on AppKit's main thread, so this only enqueues.
struct PageEvents {
    host: Arc<Mutex<Option<std::sync::mpsc::Sender<Value>>>>,
    events: RefCell<Vec<HostEvent>>,
    wake: Rc<dyn Fn()>,
}
impl SyncHandler for PageEvents {
    fn call(&self, _: &RequestContext, _: &str, _: Value) -> PageResult<Value> {
        Err(PageError::Forbidden)
    }
    fn event(&self, event: HostEvent) {
        self.events.borrow_mut().push(event);
        (self.wake)();
    }
    fn remote_dom(&self, _: &RequestContext, payload: Value) -> PageResult<()> {
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
    window: u64,
    pub host: Option<ExtensionHost>,
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
    next_menu: u64,
    placed: BTreeMap<String, layout::Rect>,
    seen: u64,
    commands: BTreeSet<String>,
    /// Mounted v4 surface extensions by title, with the source they run.
    surfaces: BTreeMap<String, (String, String)>,
    /// Contributed definitions, pinned per generation for viewers and exports.
    pub resources: Arc<ResourceSnapshot>,
}
impl Default for Extensions {
    fn default() -> Self {
        Self {
            window: NEXT_WINDOW.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            host: None,
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
            next_menu: 0,
            placed: BTreeMap::new(),
            seen: 0,
            commands: BTreeSet::new(),
            surfaces: BTreeMap::new(),
            resources: Arc::default(),
        }
    }
}
impl Drop for Extensions {
    fn drop(&mut self) {
        layout::set_contributed_panels(self.window, []);
    }
}
impl Extensions {
    pub fn owns_panel(&self, id: &str) -> bool {
        self.known_panels.contains(id)
    }
    /// The shipped app's extensions as first launch migrated them (its sealed
    /// checkpoint, kept current by each launch's delta pass), then this profile's
    /// own. The native folder wins when both have an id. The shipped app's live
    /// folder is never read: an isolated profile stays isolated.
    fn roots(profile: &std::path::Path) -> Vec<PathBuf> {
        vec![
            profile.join("extensions-shared"),
            profile.join("extensions"),
        ]
    }
}
impl Controller {
    pub fn start_extensions(&mut self, cx: &mut Context<Self>) {
        let mut shell = self.host.borrow_mut();
        shell.native_present = None;
        shell.native_accessibility = vec![];
        drop(shell);
        self.extensions = Extensions::default();
        let Some(session) = &self.session else {
            return;
        };
        let project = session.project.clone();
        let (wake_tx, wake_rx) = async_channel::bounded(1);
        let (clock_tx, clock_rx) = async_channel::bounded(64);
        let host = ExtensionHost::start(HostConfig {
            kernel: self.kernel.clone(),
            project,
            roots: Extensions::roots(&self.profile),
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
        self.extensions.host = Some(host);
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
        for toast in host.take_toasts() {
            self.message = toast;
            cx.notify();
        }
        for request in host.take_requests() {
            self.extension_request(request, cx);
        }
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
        self.present_page(cx);
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
                let answer = crate::platform::choice(&shell.window, title, body, &["OK", "Cancel"]);
                drop(shell);
                cx.spawn(async move |this, cx| {
                    let confirmed = answer.recv().await == Ok(0);
                    let _ = this.update(cx, |v, _| {
                        if let Some(host) = &v.extensions.host {
                            host.answer(token, confirmed.into());
                        }
                    });
                })
                .detach();
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
    /// Extensions add to a tab's menu asynchronously, so it opens when they
    /// answer or the deadline passes, whichever comes first.
    pub fn extension_panel_menu(
        &mut self,
        id: &str,
        entries: Vec<MenuEntry>,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) -> Option<Vec<MenuEntry>> {
        // With no realm nothing can add to it, so it opens right away.
        let Some(host) = self.extensions.host.as_ref().filter(|h| h.view() != 0) else {
            return Some(entries);
        };
        self.extensions.next_menu += 1;
        let key = self.extensions.next_menu;
        host.gather_menu("panel:context", json!({"panelId": id}), key);
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
        None
    }
    /// Each v4 extension gets its own realm and draws through the v4 panel path.
    /// One that is disabled, removed or rebuilt loses that realm here.
    fn mount_surfaces(&mut self, cx: &mut Context<Self>) {
        let (Some(host), Some(runtime)) = (&self.extensions.host, self.builtins.runtime.clone())
        else {
            return;
        };
        let wanted = host.surfaces();
        let source = |surface: &Value| surface["graph"]["hash"].as_str().map(str::to_owned);
        let gone: Vec<String> = self
            .extensions
            .surfaces
            .iter()
            .filter(|(id, (_, hash))| {
                !wanted
                    .iter()
                    .any(|s| s["id"] == id.as_str() && source(s).as_ref() == Some(hash))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &gone {
            runtime.unmount_extension(id);
            self.builtins.forget_surface(id);
            self.extensions.surfaces.remove(id);
            if !wanted.iter().any(|s| s["id"] == id.as_str()) {
                let shell = self.host.borrow();
                shell.send_command(&format!("panel:close:{id}"), vec![]);
            }
        }
        for surface in wanted {
            let Some(id) = surface["id"].as_str().map(str::to_owned) else {
                continue;
            };
            if self.extensions.surfaces.contains_key(&id) {
                continue;
            }
            let title = surface["manifest"]["name"]
                .as_str()
                .unwrap_or(&id)
                .to_owned();
            let hash = source(&surface).unwrap_or_default();
            self.extensions.surfaces.insert(id.clone(), (title, hash));
            let ticket = runtime.mount_extension(&id, &surface["graph"]);
            cx.spawn(async move |this, cx| {
                let mounted = match ticket {
                    Ok(ticket) => ticket.await.map(|_| ()),
                    Err(error) => Err(error),
                };
                let _ = this.update(cx, |v, cx| {
                    match mounted {
                        Ok(()) => v.surface_ready(id),
                        Err(error) => v.message = format!("{id} could not start: {error}"),
                    }
                    v.drain_builtins(cx);
                    cx.notify();
                });
            })
            .detach();
        }
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
            .filter(|key| !key.is_empty() && !running.contains(key))
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
                shell.send_command(&format!("panel:close:{id}"), vec![]);
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
        layout::set_contributed_panels(self.extensions.window, contributed);
        // New installs need a pinned Store key; until one ships there is no entry.
        let store = pm_compat_runner::store_install::available().then(|| Choice {
            label: "Install Extension from Store…".into(),
            command: STORE.into(),
        });
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
        let mut shell = self.host.borrow_mut();
        shell.contributed_choices = choices;
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
            self.apply_extension_resources();
        }
        self.update_builtins(cx);
        cx.notify();
    }
    /// Every live viewer renders the same contributed definitions.
    pub fn apply_extension_resources(&mut self) {
        if let Some(viewer) = &self.viewer
            && let Err(error) = viewer.resources(self.extensions.resources.clone())
        {
            self.message = error.to_string();
        }
    }
    pub fn extension_command(&mut self, cmd: &ShellCommand) -> bool {
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
        let Some(id) = cmd.name.strip_prefix(COMMAND) else {
            return false;
        };
        if let Some(host) = &self.extensions.host {
            host.run_command(id, Value::Array(cmd.args.clone()));
        }
        true
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
        let created = match &session {
            Some(session) => MacDomHost::new_store_remote_dom(
                mtm,
                session,
                events.clone(),
                &mut self.extensions.data_stores,
                PAGE_BUNDLE,
            ),
            None if key.is_empty() => MacDomHost::new_remote_dom(
                mtm,
                Trust::Trusted,
                events.clone(),
                &mut self.extensions.data_stores,
                PAGE_BUNDLE,
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
        shell.native_present = Some(Rc::new(move |width, height| {
            let viewport = Rect::new(0., 0., width, height);
            let bodies = bodies.borrow();
            let owners = owners.borrow();
            let mut dirty = false;
            for (key, page) in views.borrow().iter() {
                let mut host = page.borrow_mut();
                let regions = bodies
                    .iter()
                    .filter(|(id, _)| owners.get(id).map_or("", String::as_str) == key)
                    .map(|(_, r)| Rect::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64))
                    .collect();
                let _ = host.set_regions(regions);
                dirty |= host.composition_dirty();
                let _ = host.attach(
                    PAGE_PANEL,
                    PanelSlot {
                        id: PAGE_PANEL,
                        bounds: viewport,
                        clip: viewport,
                        reading_order: 1,
                    },
                );
                let _ = host.present(viewport);
            }
            dirty
        }));
        shell.native_accessibility = self
            .extensions
            .views
            .borrow()
            .values()
            .map(|page| page.borrow().accessibility_view())
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
                if this.update(cx, |v, cx| v.present_page(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    /// Sends placement after layout: positions live in the realm's document.
    fn place_panels(&mut self) {
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
    fn present_page(&mut self, cx: &mut Context<Self>) {
        let Some(host) = &self.extensions.host else {
            return;
        };
        for (key, page) in &mut self.extensions.pages {
            for event in page.events.events.borrow_mut().drain(..) {
                match event {
                    HostEvent::Ready { .. } => page.ready = true,
                    HostEvent::CompositionChanged => cx.notify(),
                    HostEvent::ProcessTerminated => page.ready = false,
                    HostEvent::Recovered { .. } => page.ready = true,
                    _ => (),
                }
            }
            if !page.ready {
                continue;
            }
            let mut batches = if key.is_empty() {
                host.take_batches(page.view)
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
