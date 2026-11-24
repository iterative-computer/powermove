//! The first-run film's editor: the real one, on a project and kernel of
//! its own. An in-memory kernel (no journal, no backups, nothing on any
//! profile) opens the film's project; the built-in panels run in their own
//! runtime; the pm-shell editor docks them as the app does. The film's
//! requests change it through the agent's own paths (`apply`), and the
//! stage pictures it (`pm_shell::stage::Demo`).
use crate::timeline_surface::TimelineSurface;
use crate::v4_view::{Natives, SurfaceView};
use gpui::{prelude::*, *};
use pm_builtins::{Builtins, Publication};
use pm_kernel::{KernelActor, KernelConfig, KernelHandle, session::SessionSource};
use pm_shell::{stage::Change, view::Editor};
use serde_json::Value;
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

/// The film's project id in its own kernel.
pub const PROJECT: &str = "pm-first-run-film";
/// The panels the film's workspace shows (the Design workspace's).
const PANELS: [&str; 6] = [
    "inspector",
    "fxbrowser",
    "assets",
    "timeline",
    "viewer",
    "layer-effects",
];
/// Panels that fill their slot and own their scroll (`builtin_panel`).
const FIXED: [&str; 2] = ["viewer", "timeline"];

/// The shell's integration for the film's editor: built-in panels only,
/// their bodies built as the app builds them.
pub(crate) struct Dock {
    pub runtime: Arc<Builtins>,
    pub tokens: pm_design::Tokens,
    pub natives: Natives,
    pub store: Rc<pm_shell::workspace::Store>,
    pub pending: RefCell<HashMap<String, Publication>>,
    pub views: RefCell<HashMap<String, Entity<SurfaceView>>>,
    pub scrolls: RefCell<HashMap<String, ScrollHandle>>,
}
impl pm_shell::integration::Integration for Dock {
    fn observe(&self, _: &mut Context<Editor>) {}
    fn command(&self, _: &pm_shell::Command, _: &mut Window, _: &mut App) -> bool {
        false
    }
    fn workspaces(&self, _: &App) -> Option<Rc<pm_shell::workspace::Store>> {
        Some(self.store.clone())
    }
    fn project_id(&self, _: &App) -> Option<String> {
        Some(PROJECT.into())
    }
    fn status(&self, _: &App) -> String {
        String::new()
    }
    fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    fn paints_own_ground(&self, id: &str, _: &App) -> bool {
        id == "timeline"
    }
    fn natural_panel_height(&self, id: &str, cx: &App) -> Option<f32> {
        let height = self.views.borrow().get(id)?.read(cx).natural_body_height();
        (height > 0.).then_some(height + self.tokens.metrics.header)
    }
    fn panel(
        &self,
        id: &str,
        rect: pm_shell::layout::Rect,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if let Some(first) = self.pending.borrow_mut().remove(id) {
            let view = cx.new(|cx| {
                SurfaceView::new(
                    self.runtime.clone(),
                    first,
                    self.tokens.clone(),
                    self.natives.clone(),
                    window,
                    cx,
                )
            });
            if !FIXED.contains(&id) {
                view.update(cx, |v, _| v.grows = true);
            }
            if id == "timeline" {
                self.natives.outline.bind(view.downgrade());
            }
            self.views.borrow_mut().insert(id.into(), view);
        }
        let view = self.views.borrow().get(id).cloned()?;
        view.update(cx, |v, cx| v.attach(window, cx));
        let body = (rect.h - self.tokens.metrics.header).max(0.);
        if FIXED.contains(&id) {
            return Some(
                div()
                    .w_full()
                    .h(px(body))
                    .overflow_hidden()
                    .child(crate::builtins::fixed_body_view(id, &view))
                    .into_any_element(),
            );
        }
        let handle = self
            .scrolls
            .borrow_mut()
            .entry(id.into())
            .or_default()
            .clone();
        view.update(cx, |v, _| v.scrolls_in(&handle));
        let content = view.read(cx).in_body(body);
        Some(
            div()
                .relative()
                .w_full()
                .h(px(body))
                .child(
                    div()
                        .id(SharedString::from(format!("film-panel-body-{id}")))
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&handle)
                        .scroll_edge_fade_y(px(pm_design::DOCK_SCROLL_FADE))
                        .child(view.clone().cached(content)),
                )
                .into_any_element(),
        )
    }
}

/// The film's timeline asks nothing of a host: its playhead is the
/// film's, and there are no menus, drops or commands in a film.
#[derive(Default)]
struct FilmTimeline {
    time: std::cell::Cell<f64>,
}
impl crate::timeline_surface::Delegate for FilmTimeline {
    fn time(&self, _: &App) -> f64 {
        self.time.get()
    }
    fn seek(&self, seconds: f64, _: &mut App) {
        self.time.set(seconds);
    }
    fn effect(&self, _: pm_panel_timeline::HostEffect, _: &mut App) {}
    fn report(&self, _: String, _: &mut App) {}
    fn command(&self, _: &str, _: &mut App) {}
    fn menu(&self, _: Vec<Value>, _: &mut Window, _: &mut App, _: crate::timeline_surface::Choose) {
    }
}

/// What the film's editor starts with, made off the main thread: the
/// kernel, the agent's tools on a temporary profile, and the panels
/// mounting.
struct Started {
    kernel: KernelHandle,
    runtime: Builtins,
    tools: crate::mcp::Connection,
    automation: crate::mcp::Tools,
    actor: KernelActor,
    profile: tempfile::TempDir,
    mounting: Vec<(String, pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>)>,
}
fn start(project: Value) -> anyhow::Result<Started> {
    let profile = tempfile::Builder::new().prefix("pm-film-").tempdir()?;
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    crate::mcp::attach(&kernel, profile.path())?;
    let host = crate::mcp::Host::new(kernel.clone(), profile.path().to_owned());
    host.open(
        PROJECT.into(),
        SessionSource::New(pm_model::json::parse(&project.to_string())?),
    )?;
    if let Some(current) = host.current() {
        host.save(current, None)?;
    }
    let automation = crate::mcp::Tools::new(host, profile.path().join("exports"));
    automation.automation.lock().unwrap().extensions = Some(Arc::new(
        crate::extension_authoring::AppExtensions::new(profile.path().to_owned()),
    ));
    let tools = automation.connect();
    let mut runtime = Builtins::new(kernel.clone(), PROJECT, || {})?;
    // The film's agent: an API or local model connection whose host sends
    // nothing (the film never sends a request). No runtime is looked for
    // and no sign-in is read.
    runtime.set_agent_environment(
        pm_agents::launch::Environment {
            profile: profile.path().to_owned(),
            resources: None,
            app_version: env!("CARGO_PKG_VERSION").into(),
            executable: std::path::PathBuf::new(),
            vars: Default::default(),
            tools: vec![],
            composition_tools: vec![],
            compatible: Some(Arc::new(FilmChat)),
        },
        Arc::new(|_: &str| Err("The film signs in to nothing.".into())),
    );
    let mut mounting = Vec::new();
    for panel in PANELS.iter().chain(["agent"].iter()) {
        mounting.push(((*panel).to_owned(), runtime.mount(panel)?));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    mounting.retain(|(panel, ticket)| {
        loop {
            match ticket.try_take() {
                Some(Ok(_)) => return false,
                Some(Err(error)) => {
                    eprintln!("The film's {panel} panel could not start: {error}");
                    return false;
                }
                None if std::time::Instant::now() > deadline => return true,
                None => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        }
    });
    // The panel's own provider choice, as its menu sends it.
    choose_film_provider(&runtime)?;
    Ok(Started {
        kernel,
        runtime,
        tools,
        automation,
        actor,
        profile,
        mounting,
    })
}

/// The film's model connection: saved, so the agent panel is ready, but
/// it refuses every request (the film types; it never sends).
#[derive(Debug)]
struct FilmChat;
impl pm_agents::compatible::ChatHost for FilmChat {
    fn saved(&self) -> Option<pm_agents::compatible::Saved> {
        Some(pm_agents::compatible::Saved {
            base_url: String::new(),
            // OWNER-COPY: the model name the film's composer shows.
            model: "Your agent".into(),
            vision: false,
            models: vec![],
        })
    }
    fn key(&self, _: &str) -> Option<String> {
        None
    }
    fn stream(
        &self,
        _: &str,
        _: Option<&str>,
        _: Vec<u8>,
        _: &std::sync::atomic::AtomicBool,
        _: &mut dyn FnMut(&[u8]) -> Result<(), String>,
    ) -> Result<u16, String> {
        Err("The film sends nothing.".into())
    }
}
/// Waits for a panel command's answer (on a worker).
fn answer(ticket: pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>) -> anyhow::Result<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(result) = ticket.try_take() {
            let value = result?.value;
            return Ok(serde_json::from_str(&value).unwrap_or(Value::String(value)));
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "the agent panel did not answer"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
/// Switches the film's agent panel to its model connection through the
/// panel's own dispatch, as its provider menu does.
fn choose_film_provider(runtime: &Builtins) -> anyhow::Result<()> {
    let sequence = runtime.agent_sequence().map_err(anyhow::Error::msg)?;
    let thread = runtime.agent_current_thread();
    let envelope = serde_json::json!({
        "sequence": sequence,
        "target": {"project": PROJECT, "thread": thread, "run": null},
        "action": {"type": "setProvider", "provider": "compatible", "model": "Your agent"},
    });
    answer(runtime.panel_command("agent", "panel.agent.dispatch", &envelope)?)?;
    Ok(())
}

/// The film's editor: the agent's MCP tools on a kernel and profile of the
/// film's own, the built-in panels, the pm-shell editor docking them, and
/// the agent panel whose composer the film types into.
pub struct DemoEditor {
    pub kernel: KernelHandle,
    pub runtime: Arc<Builtins>,
    pub tokens: pm_design::Tokens,
    /// The agent's own MCP connection, on the film's script worker.
    script: std::sync::mpsc::Sender<(Change, std::sync::mpsc::Sender<Result<(), String>>)>,
    _automation: crate::mcp::Tools,
    _actor: KernelActor,
    /// Everything the film writes (library, extensions, runs) lives here
    /// and goes with it.
    pub profile: std::path::PathBuf,
    _profile: tempfile::TempDir,
    mounting: Vec<(String, pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>)>,
    pub(crate) dock: Option<Rc<Dock>>,
    editor: Option<Entity<Editor>>,
    timeline: Option<Entity<TimelineSurface>>,
    agent: Option<Entity<SurfaceView>>,
    agent_first: Option<Publication>,
    typed: String,
}
impl DemoEditor {
    fn new(started: Started, tokens: pm_design::Tokens) -> Self {
        let Started {
            kernel,
            runtime,
            tools,
            automation,
            actor,
            profile,
            mounting,
        } = started;
        let path = profile.path().to_owned();
        // The agent's calls block on the kernel: they run on a worker, one
        // change at a time, in order.
        let (script, jobs) =
            std::sync::mpsc::channel::<(Change, std::sync::mpsc::Sender<Result<(), String>>)>();
        let script_profile = path.clone();
        std::thread::Builder::new()
            .name("film-script".into())
            .spawn(move || {
                let mut tools = tools;
                while let Ok((change, reply)) = jobs.recv() {
                    let result = crate::film_script::apply(&mut tools, &script_profile, change)
                        .map_err(|e| format!("{e:#}"));
                    let _ = reply.send(result);
                }
            })
            .expect("the film's script worker starts");
        Self {
            kernel,
            runtime: Arc::new(runtime),
            tokens,
            script,
            _automation: automation,
            _actor: actor,
            profile: path,
            _profile: profile,
            mounting,
            dock: None,
            editor: None,
            timeline: None,
            agent: None,
            agent_first: None,
            typed: String::new(),
        }
    }
    /// Asks the script worker for `change`; the answer comes on the receiver.
    pub fn apply(&self, change: Change) -> std::sync::mpsc::Receiver<Result<(), String>> {
        let (reply, answer) = std::sync::mpsc::channel();
        if self.script.send((change, reply.clone())).is_err() {
            let _ = reply.send(Err("the film's script worker stopped".into()));
        }
        answer
    }
    /// Whether every panel has mounted.
    pub fn mounted(&mut self) -> bool {
        self.mounting
            .retain(|(panel, ticket)| match ticket.try_take() {
                Some(Ok(_)) => false,
                Some(Err(error)) => {
                    eprintln!("The film's {panel} panel could not start: {error}");
                    false
                }
                None => true,
            });
        self.mounting.is_empty()
    }
    /// Each panel's new publication to its view (or kept for a panel not
    /// drawn yet).
    fn drain(&mut self, cx: &mut App) {
        let Some(dock) = self.dock.clone() else {
            return;
        };
        for p in self.runtime.take_publications() {
            if p.panel == "agent" {
                match &self.agent {
                    Some(view) => view.update(cx, |v, cx| v.publish(p, cx)),
                    None => self.agent_first = Some(p),
                }
                continue;
            }
            let view = dock.views.borrow().get(&p.panel).cloned();
            match view {
                Some(view) => view.update(cx, |v, cx| v.publish(p, cx)),
                None => {
                    dock.pending.borrow_mut().insert(p.panel.clone(), p);
                }
            }
        }
        for error in self.runtime.take_errors() {
            eprintln!("film: {error}");
        }
    }
    /// The editor, built once the panels have mounted and published.
    pub fn editor(&mut self, cx: &mut App) -> Option<Entity<Editor>> {
        if self.editor.is_none() {
            if !self.mounted() {
                return None;
            }
            let published = self.kernel.published(PROJECT)?;
            let natives = Natives::default();
            let timeline_tokens = crate::builtins::timeline_chrome(
                self.tokens.clone(),
                &published.workspace,
                &Value::Null,
            );
            let timeline = cx.new(|cx| {
                let mut t = TimelineSurface::new(
                    self.kernel.clone(),
                    PROJECT.into(),
                    timeline_tokens,
                    Rc::new(FilmTimeline::default()),
                    natives.outline.clone(),
                    Default::default(),
                    cx,
                );
                t.set_outline_runtime(&self.runtime);
                t
            });
            natives
                .views
                .borrow_mut()
                .insert("timeline".into(), timeline.clone().into());
            let mut pending: HashMap<String, Publication> = self
                .runtime
                .take_publications()
                .into_iter()
                .map(|p| (p.panel.clone(), p))
                .collect();
            self.agent_first = pending.remove("agent").or(self.agent_first.take());
            let dock = Rc::new(Dock {
                runtime: self.runtime.clone(),
                tokens: self.tokens.clone(),
                natives,
                store: Rc::new(pm_shell::workspace::Store {
                    records: pm_shell::workspace::seeded(&[]),
                    project_id: Some(PROJECT.into()),
                    ..Default::default()
                }),
                pending: RefCell::new(pending),
                views: RefCell::default(),
                scrolls: RefCell::default(),
            });
            let integration = dock.clone();
            let tokens = self.tokens.clone();
            let editor = cx.new(|cx| {
                let mut editor = Editor::new(tokens, None, cx);
                editor.integration = Some(integration);
                editor
            });
            self.dock = Some(dock);
            self.timeline = Some(timeline);
            self.editor = Some(editor);
        }
        self.drain(cx);
        self.editor.clone()
    }
    /// The agent panel, whose composer the film types into.
    pub fn agent(&mut self, window: &mut Window, cx: &mut App) -> Option<Entity<SurfaceView>> {
        if self.agent.is_none() {
            let first = self.agent_first.take()?;
            let dock = self.dock.as_ref()?;
            let view = cx.new(|cx| {
                SurfaceView::new(
                    self.runtime.clone(),
                    first,
                    self.tokens.clone(),
                    dock.natives.clone(),
                    window,
                    cx,
                )
            });
            self.agent = Some(view);
        }
        let view = self.agent.clone()?;
        view.update(cx, |v, cx| v.attach(window, cx));
        Some(view)
    }
    /// Types `text` into the composer, as a person's typing would arrive
    /// (the panel's own fill; the request is never sent).
    pub fn type_into_composer(&mut self, text: &str, window: &mut Window, cx: &mut App) {
        if self.typed == text {
            return;
        }
        let Some(view) = self.agent.clone() else {
            return;
        };
        let _ = window;
        // The field shows the text as a person's typing does.
        if !view.update(cx, |v, cx| v.type_composer(text, cx)) {
            return;
        }
        self.typed = text.to_owned();
    }
}
impl Drop for DemoEditor {
    fn drop(&mut self) {
        self.runtime.close();
    }
}

/// The engine's pictures, shared by the stage (the room) and the film's
/// viewer (the project).
#[derive(Clone)]
pub struct SharedPictures(pub Rc<RefCell<crate::stage_pictures::EnginePictures>>);
impl pm_shell::stage::Pictures for SharedPictures {
    fn picture(
        &mut self,
        what: pm_shell::stage::Picture,
        t: f64,
        size: (u32, u32),
    ) -> Option<Arc<gpui::NativeTexture>> {
        self.0.borrow_mut().picture(what, t, size)
    }
    fn rest(&mut self) {
        self.0.borrow_mut().rest();
    }
}

/// The film's viewer picture: the engine's frame of the film's project,
/// fitted in the viewer's picture area as the viewer fits it.
struct FilmViewer {
    pictures: SharedPictures,
    time: Rc<std::cell::Cell<f64>>,
    aspect: f32,
}
impl Render for FilmViewer {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut pictures = self.pictures.clone();
        let t = self.time.get();
        let aspect = self.aspect;
        let scale = window.scale_factor();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                let (fw, fh) = if w / h > aspect {
                    (h * aspect, h)
                } else {
                    (w, w / aspect)
                };
                let frame = Bounds::new(
                    bounds.origin + point(px((w - fw) / 2.), px((h - fh) / 2.)),
                    size(px(fw), px(fh)),
                );
                let device = (((fw * scale) as u32).max(1), ((fh * scale) as u32).max(1));
                use pm_shell::stage::Pictures;
                if let Some(texture) = pictures.picture(pm_shell::stage::Picture::Viewer, t, device)
                {
                    window.paint_texture(frame, texture);
                }
            },
        )
        .size_full()
    }
}

/// The film's side of the stage: the real editor and composer, and each
/// request applied through the agent's own tools.
pub struct FilmDemo {
    tokens: pm_design::Tokens,
    starting: Option<std::sync::mpsc::Receiver<anyhow::Result<Started>>>,
    editor: Option<DemoEditor>,
    pictures: SharedPictures,
    /// The film project's playhead (the viewer's picture and the timeline).
    time: Rc<std::cell::Cell<f64>>,
    viewer: Option<AnyView>,
    asked: HashMap<Change, std::sync::mpsc::Receiver<Result<(), String>>>,
    applied: HashMap<Change, Result<(), String>>,
}
impl FilmDemo {
    /// Starts the film's editor on a worker (it blocks on its kernel).
    pub fn start(project: Value, tokens: pm_design::Tokens, pictures: SharedPictures) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("film-editor".into())
            .spawn(move || {
                let _ = tx.send(start(project));
            })
            .expect("the film's editor starts");
        Self {
            tokens,
            starting: Some(rx),
            editor: None,
            pictures,
            time: Rc::default(),
            viewer: None,
            asked: HashMap::new(),
            applied: HashMap::new(),
        }
    }
    /// Why a change could not be made, if one could not.
    pub fn failures(&self) -> Vec<(Change, String)> {
        self.applied
            .iter()
            .filter_map(|(c, r)| r.as_ref().err().map(|e| (*c, e.clone())))
            .collect()
    }
    fn started(&mut self, wait: bool) -> Option<&mut DemoEditor> {
        if let Some(rx) = &self.starting {
            let started = if wait {
                rx.recv().ok()
            } else {
                rx.try_recv().ok()
            };
            if let Some(started) = started {
                self.starting = None;
                match started {
                    Ok(started) => {
                        self.editor = Some(DemoEditor::new(started, self.tokens.clone()))
                    }
                    Err(error) => eprintln!("The film's editor could not start: {error:#}"),
                }
            }
        }
        self.editor.as_mut()
    }
    fn collect(&mut self, wait: bool) {
        let changes: Vec<Change> = self.asked.keys().copied().collect();
        for change in changes {
            let rx = &self.asked[&change];
            let answer = if wait {
                rx.recv().ok()
            } else {
                rx.try_recv().ok()
            };
            if let Some(result) = answer {
                if let Err(error) = &result {
                    eprintln!("The film's change {change:?} failed: {error}");
                }
                self.applied.insert(change, result);
                self.asked.remove(&change);
            }
        }
    }
}
impl pm_shell::stage::Demo for FilmDemo {
    fn editor(&mut self, window: &mut Window, cx: &mut App) -> AnyElement {
        self.collect(false);
        let ground = self.tokens.colors.window;
        let Some(editor) = self.started(false).and_then(|e| e.editor(cx)) else {
            return div().size_full().bg(rgb(ground)).into_any_element();
        };
        if self.viewer.is_none()
            && let Some(dock) = self.editor.as_ref().and_then(|e| e.dock.clone())
        {
            let viewer = cx.new(|_| FilmViewer {
                pictures: self.pictures.clone(),
                time: self.time.clone(),
                aspect: 16. / 9.,
            });
            dock.natives
                .views
                .borrow_mut()
                .insert("viewer.overlays".into(), viewer.clone().into());
            self.viewer = Some(viewer.into());
        }
        let _ = window;
        div()
            .size_full()
            .bg(rgb(ground))
            .child(editor)
            .into_any_element()
    }
    fn composer(
        &mut self,
        composer: &pm_shell::stage::Composer,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let Some(editor) = self.started(false) else {
            return div().into_any_element();
        };
        let _ = editor.editor(cx);
        let Some(agent) = editor.agent(window, cx) else {
            return div().into_any_element();
        };
        editor.type_into_composer(&composer.text, window, cx);
        // The panel's composer sits at its foot: the box shows that much.
        div()
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .justify_end()
            .child(div().flex_none().w_full().h(px(420.)).child(agent))
            .into_any_element()
    }
    fn composer_home(&self, area: Size<Pixels>) -> Bounds<Pixels> {
        let (w, h) = (px(560.), px(132.));
        Bounds::new(
            point((area.width - w) / 2., area.height - h - px(36.)),
            size(w, h),
        )
    }
    fn apply(&mut self, change: Change, _cx: &mut App) {
        if let Some(editor) = self.started(false) {
            let answer = editor.apply(change);
            self.asked.insert(change, answer);
        } else {
            self.applied
                .insert(change, Err("the film's editor had not started".into()));
        }
    }
    fn landed(&self, change: Change, _cx: &App) -> bool {
        self.applied.contains_key(&change)
    }
    fn site(&self, _change: Change, area: Size<Pixels>) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), area)
    }
    fn at(&mut self, t: f64) {
        // The film's project plays from when the editor arrives, looping.
        let start = pm_shell::stage::CueSheet::default()
            .start_of(pm_shell::stage::Scene::Beat(Change::GlassTimeline))
            .unwrap_or(0.);
        self.time.set((t - start).max(0.) % 8.);
    }
    fn settle(&mut self, cx: &mut App) {
        self.collect(true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        if let Some(editor) = self.started(true) {
            while !editor.mounted() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let _ = editor.editor(cx);
        }
    }
}
