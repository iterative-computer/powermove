//! The running app's v1–3 extensions. `ExtensionHost` owns V8 on its own thread;
//! this side only reads published copies, routes page messages and places the
//! one WebKit document below GPUI. Nothing here can wait on extension code.
use crate::session::Controller;
use gpui::{prelude::*, *};
use objc2::MainThreadMarker;
use pm_engine_service::resources::ResourceSnapshot;
use pm_compat_host::{
    DomPanelHost, Error as PageError, HostEvent, MacDomHost, PanelSlot, PanelSpec, Rect,
    RequestContext, Result as PageResult, StoreDataStores, SyncHandler, Trust,
};
use pm_compat_runner::{
    PAGE_BUNDLE,
    host::{ExtensionHost, HostConfig},
    transport::{TransportOwner, TransportRequest},
};
use pm_node::fonts::render::BrokerResolver;
use pm_protocol::{clock::TransportCommand, common::PlaybackRate};
use pm_shell::{Command as ShellCommand, keyboard::Binding, layout, transient::Choice};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
};

const PAGE_PANEL: u64 = 1;
const COMMAND: &str = "extension.command:";
const OWNER: &str = "extension:";

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
}

/// The realm asks the one clock owner to move; the Controller answers on main.
struct Clock(async_channel::Sender<TransportRequest>);
impl TransportOwner for Clock {
    fn request(&self, request: TransportRequest) {
        let _ = self.0.try_send(request);
    }
}

#[derive(Default)]
pub struct Extensions {
    pub host: Option<ExtensionHost>,
    page: Option<Page>,
    /// Open contributed panels and where the shell placed their bodies.
    panels: BTreeMap<String, layout::Rect>,
    known_panels: BTreeSet<String>,
    placed: BTreeMap<String, layout::Rect>,
    seen: u64,
    commands: BTreeSet<String>,
    /// Mounted v4 surface extensions and their titles.
    surfaces: BTreeMap<String, String>,
    /// Contributed definitions, pinned per generation for viewers and exports.
    pub resources: Arc<ResourceSnapshot>,
}
impl Extensions {
    pub fn owns_panel(&self, id: &str) -> bool {
        self.known_panels.contains(id)
    }
    /// Extensions from this profile, then the shipped app's, so nothing is left
    /// behind on update. The native folder wins when both have an id.
    fn roots(profile: &std::path::Path) -> Vec<PathBuf> {
        let mut roots = vec![];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Library/Application Support/Powermove/extensions"));
        }
        roots.push(profile.join("extensions"));
        roots
    }
}
impl Controller {
    pub fn start_extensions(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else {
            self.extensions = Extensions::default();
            layout::set_contributed_panels([]);
            return;
        };
        let project = session.project.clone();
        self.extensions = Extensions::default();
        let (wake_tx, wake_rx) = async_channel::bounded(1);
        let (clock_tx, clock_rx) = async_channel::bounded(64);
        let host = ExtensionHost::start(HostConfig {
            kernel: self.kernel.clone(),
            project,
            roots: Extensions::roots(&self.profile),
            cache: self.profile.join("extensions-build"),
            owner: Some(Box::new(Clock(clock_tx))),
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
        if revision != self.extensions.seen {
            self.extensions.seen = revision;
            self.mount_surfaces(cx);
            if let Some(contributions) = self.extensions.host.as_ref().and_then(|h| h.contributions()) {
                self.apply_contributions(&contributions, cx);
            }
        }
        self.present_page(cx);
    }
    /// Each v4 extension gets its own realm and draws through the v4 panel path.
    fn mount_surfaces(&mut self, cx: &mut Context<Self>) {
        let (Some(host), Some(runtime)) = (&self.extensions.host, self.builtins.runtime.clone())
        else {
            return;
        };
        for surface in host.surfaces() {
            let Some(id) = surface["id"].as_str().map(str::to_owned) else {
                continue;
            };
            if self.extensions.surfaces.contains_key(&id) {
                continue;
            }
            let title = surface["manifest"]["name"].as_str().unwrap_or(&id).to_owned();
            self.extensions.surfaces.insert(id.clone(), title);
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
        self.extensions.known_panels = panels.iter().map(|(id, _)| id.clone()).collect();
        let surfaces = self.extensions.surfaces.clone().into_iter();
        layout::set_contributed_panels(panels.into_iter().chain(surfaces));
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
            .collect();
        self.extensions.commands = choices
            .iter()
            .map(|c| c.command[COMMAND.len()..].to_owned())
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
        if self.extensions.panels.insert(id.to_owned(), body).is_none()
            && let Some(host) = &self.extensions.host
        {
            host.set_panel(id, true);
        }
        self.ensure_page(cx);
        self.place_panels();
        Some(
            div()
                .w_full()
                .h(px(body.h))
                .id(SharedString::from(format!("extension-panel-{id}")))
                .into_any_element(),
        )
    }
    fn ensure_page(&mut self, cx: &mut Context<Self>) {
        if self.extensions.page.is_some() {
            return;
        }
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
        let created = MacDomHost::new_remote_dom(
            mtm,
            Trust::Trusted,
            events.clone(),
            &mut StoreDataStores::default(),
            PAGE_BUNDLE,
        );
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
        let present = page.clone();
        let mut shell = self.host.borrow_mut();
        let bodies = shell.native_panels.clone();
        shell.native_present = Some(Rc::new(move |width, height| {
            let mut host = present.borrow_mut();
            let viewport = Rect::new(0., 0., width, height);
            let regions = bodies
                .borrow()
                .iter()
                .map(|r| Rect::new(r.x as f64, r.y as f64, r.w as f64, r.h as f64))
                .collect();
            let _ = host.set_regions(regions);
            let dirty = host.composition_dirty();
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
            dirty
        }));
        shell.native_accessibility = vec![page.borrow().accessibility_view()];
        drop(shell);
        self.extensions.page = Some(Page {
            host: page,
            events,
            ready: false,
        });
        // Feedback reaches the realm on a worker, never inside the page's callback.
        let sink = self.extensions.host.as_ref().map(|h| h.page_sink());
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
            if self.extensions.placed.get(id) != Some(rect) {
                host.place_panel(id, [rect.x, rect.y, rect.w, rect.h]);
            }
        }
        self.extensions.placed = self.extensions.panels.clone();
    }
    /// The workspace closed a contributed panel: unmount it in the realm.
    pub fn sync_extension_panels(&mut self, open: &[&str]) {
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
        if self.extensions.panels.remove(id).is_some()
            && let Some(host) = &self.extensions.host
        {
            host.set_panel(id, false);
        }
    }
    fn present_page(&mut self, cx: &mut Context<Self>) {
        let (Some(page), Some(host)) = (&mut self.extensions.page, &self.extensions.host) else {
            return;
        };
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
            return;
        }
        let mut batches = host.take_batches();
        let mut web = page.host.borrow_mut();
        while let Some(batch) = batches.front() {
            if batch.present(&mut *web).is_err() {
                break;
            }
            batches.pop_front();
        }
        host.return_batches(batches);
        for frame in host.take_canvases() {
            let _ = web.present_canvas(PAGE_PANEL, &frame);
        }
    }
}
