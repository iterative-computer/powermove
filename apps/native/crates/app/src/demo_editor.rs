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
use pm_compat_runner::host::{ExtensionHost, HostConfig};
use pm_kernel::{KernelActor, KernelConfig, KernelHandle, session::SessionSource};
use pm_shell::{stage::Change, view::Editor};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::{Arc, mpsc},
};

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
    /// Panels the agent's extensions contribute (mounted), id and title.
    pub contributed: RefCell<Vec<(String, String)>>,
}
impl pm_shell::integration::Integration for Dock {
    fn contributed_panels(&self, _: &App) -> Vec<(String, String)> {
        self.contributed.borrow().clone()
    }
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
struct FilmTimeline {
    time: std::cell::Cell<f64>,
    /// The film's panels: extension clip painters run in their realms.
    runtime: Arc<Builtins>,
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
    fn painters_revision(&self, _: &App) -> u64 {
        self.runtime.painters_revision()
    }
    fn paint_clips(&self, clips: Value, done: crate::timeline_surface::PaintDone, cx: &mut App) {
        crate::builtins::paint_clips(self.runtime.clone(), clips, done, cx);
    }
}

/// What the film's editor starts with, made off the main thread: the
/// kernel, the agent's tools on a temporary profile, and the panels
/// mounting.
struct Started {
    /// The agent's panel and layout tools, answered on the main thread
    /// against the film's editor (`surfaces`).
    calls: async_channel::Receiver<crate::panel_bridge::Call>,
    kernel: KernelHandle,
    runtime: Builtins,
    tools: crate::mcp::Connection,
    automation: crate::mcp::Tools,
    actor: KernelActor,
    profile: tempfile::TempDir,
    mounting: Vec<(String, pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>)>,
    /// The film's extension host: what the agent publishes runs here, on
    /// the film's kernel (v1–3 in its realm, v4 surfaces listed for the
    /// panels' runtime), as a window's host runs the user's.
    extensions: ExtensionHost,
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
    let (bridge, calls) = crate::panel_bridge::channel();
    {
        let mut a = automation.automation.lock().unwrap();
        a.extensions = Some(Arc::new(crate::extension_authoring::AppExtensions::new(
            profile.path().to_owned(),
        )));
        a.surfaces = Some(Arc::new(bridge));
    }
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
    let extensions = ExtensionHost::start(HostConfig {
        kernel: kernel.clone(),
        project: PROJECT.into(),
        roots: crate::extensions::roots(profile.path()),
        cache: profile.path().join("extensions-build"),
        owner: None,
        text: None,
        frames: None,
        rasters: None,
        vars: None,
        clipboard: None,
        // The stage draws every frame while the film runs: it looks.
        wake: Arc::new(|| {}),
    });
    Ok(Started {
        extensions,
        calls,
        kernel,
        runtime,
        tools,
        automation,
        actor,
        profile,
        mounting,
    })
}

/// One of the agent's panel or layout tools, on the film's editor: the
/// app's own handlers where they need only the editor.
fn film_surface(call: crate::panel_bridge::Call, editor: WeakEntity<Editor>, cx: &mut App) {
    let Some(editor) = editor.upgrade() else {
        call.answer(Err(pm_mcp::Error::unavailable("The film has ended.")));
        return;
    };
    let args = call.args();
    let result = match call.tool() {
        "set_panel_layout" => crate::panel_bridge::set_panel_layout(&editor, args, cx),
        "panel_begin" => crate::panel_bridge::begin_panel_slot(&editor, args, cx).map(|slot| {
            editor.update(cx, |_, cx| cx.notify());
            slot.answer()
        }),
        "panel_build" => film_panel_build(&editor, args, cx),
        // The film opens only what its layout docks (a built panel is).
        "open_panel" => {
            let panel = args["panelId"].as_str().unwrap_or_default();
            let e = editor.read(cx);
            if !e.panels(cx).contains(panel) {
                Err(pm_mcp::Error::invalid("Unknown panel ID."))
            } else if e.layout.find(panel).is_none() {
                Err(pm_mcp::Error::invalid(
                    "The film's editor opens docked panels only.",
                ))
            } else {
                Ok(serde_json::json!({"panelId": panel, "open": true}))
            }
        }
        tool => Err(pm_mcp::Error::invalid(format!(
            "The film's editor has no {tool}."
        ))),
    };
    call.answer(result);
}
/// `panel_build` on the film's editor: plan and commit are accepted (the
/// film draws no build sketch); ready docks the published panel in the slot;
/// cancel and error give the slot up.
fn film_panel_build(editor: &Entity<Editor>, args: &Value, cx: &mut App) -> pm_mcp::Result<Value> {
    let id = args["build_id"].as_str().unwrap_or_default();
    let event = args["event"].as_str().unwrap_or_default();
    let target = args["panelId"].as_str().unwrap_or_default();
    if !id.starts_with("panel-build-") || editor.read(cx).layout.find(id).is_none() {
        return Err(pm_mcp::Error::invalid("Unknown build_id."));
    }
    match event {
        "plan" => {}
        "commit" if target.is_empty() => {}
        "commit" => crate::panel_bridge::ready_target(editor, id, target, cx)?,
        "ready" => {
            crate::panel_bridge::ready_target(editor, id, target, cx)?;
            crate::panel_bridge::dock_built_panel(editor, id, target, cx);
        }
        "cancel" | "error" => editor.update(cx, |e, cx| {
            e.release_agent_panel(id);
            cx.notify();
        }),
        _ => return Err(pm_mcp::Error::invalid("Unknown panel_build event.")),
    }
    Ok(serde_json::json!({"build_id": id, "event": event, "accepted": true}))
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
    calls: Option<async_channel::Receiver<crate::panel_bridge::Call>>,
    /// The same calls, answered while a recording waits for a change.
    waiting_calls: async_channel::Receiver<crate::panel_bridge::Call>,
    extensions: ExtensionHost,
    /// Mounted extension surfaces: id and the graph hash they run.
    surfaces: HashMap<String, String>,
    /// Surfaces mounting on a worker: (id, hash, title) and the result.
    mounting_surfaces: Vec<(String, String, String, mpsc::Receiver<Result<(), String>>)>,
    /// The host publication last read, and the timeline chrome last applied.
    seen_revision: Option<u64>,
    chrome: Option<String>,
    /// What the viewer pictures: the project revision and definitions
    /// generation it was given (none: the film's own document).
    viewer: Option<(u64, u64)>,
    resources: Arc<pm_engine_service::resources::ResourceSnapshot>,
    pictures: SharedPictures,
    /// The film project's playhead, and the beat the panels were told.
    time: Rc<Cell<f64>>,
    told_beat: Option<i64>,
    /// The project revision the film opened with.
    seen_first: Option<u64>,
}
impl DemoEditor {
    fn new(
        started: Started,
        tokens: pm_design::Tokens,
        pictures: SharedPictures,
        time: Rc<Cell<f64>>,
    ) -> Self {
        let Started {
            extensions,
            calls,
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
            waiting_calls: calls.clone(),
            calls: Some(calls),
            extensions,
            surfaces: HashMap::new(),
            mounting_surfaces: Vec::new(),
            seen_revision: None,
            chrome: None,
            viewer: None,
            resources: Default::default(),
            pictures,
            time,
            told_beat: None,
            seen_first: None,
        }
    }
    /// Answers the agent's panel and layout calls waiting now (recordings,
    /// whose main thread waits for a change to land).
    fn answer_calls(&mut self, cx: &mut App) {
        let Some(editor) = self.editor.clone() else {
            return;
        };
        while let Ok(call) = self.waiting_calls.try_recv() {
            film_surface(call, editor.downgrade(), cx);
        }
    }
    /// Whether the extension `id` runs in the film (mounted, for a v4 one).
    pub fn mounted_extension(&self, id: &str) -> bool {
        self.surfaces.contains_key(id)
    }
    /// Whether the viewer draws with the definition `id` (a transition or
    /// effect an extension registered).
    pub fn draws_with(&self, id: &str) -> bool {
        self.viewer.is_some()
            && self
                .resources
                .definitions
                .iter()
                .any(|d| d.id.as_str() == id)
    }
    /// What the agent's extensions changed, taken in on the main thread
    /// without waiting: new v4 surfaces mount on a worker (their panels and
    /// clip painters), the timeline's chrome follows the workspace, the
    /// viewer draws the project as it is now with what the extensions
    /// define, and running extensions hear the playhead each beat.
    fn take_extensions(&mut self, cx: &mut App) {
        let publication = self.extensions.publication();
        if self.seen_revision != Some(publication.revision) {
            self.seen_revision = Some(publication.revision);
            for surface in self.extensions.surfaces() {
                let (Some(id), Some(hash)) =
                    (surface["id"].as_str(), surface["graph"]["hash"].as_str())
                else {
                    continue;
                };
                if self.surfaces.get(id).map(String::as_str) == Some(hash)
                    || self.mounting_surfaces.iter().any(|m| m.0 == id)
                {
                    continue;
                }
                let title = surface["manifest"]["name"]
                    .as_str()
                    .unwrap_or(id)
                    .to_owned();
                let (tx, rx) = mpsc::channel();
                let (runtime, id_, graph) = (
                    self.runtime.clone(),
                    id.to_owned(),
                    surface["graph"].clone(),
                );
                let replacing = self.surfaces.contains_key(id);
                std::thread::Builder::new()
                    .name("film-extension".into())
                    .spawn(move || {
                        if replacing {
                            runtime.unmount_extension(&id_);
                        }
                        let result = runtime
                            .mount_extension(&id_, &graph, None)
                            .map_err(|e| e.to_string())
                            .and_then(|ticket| {
                                let deadline =
                                    std::time::Instant::now() + std::time::Duration::from_secs(20);
                                crate::ticket::answer(ticket, deadline)
                                    .ok_or_else(|| "it did not finish mounting".to_owned())?
                                    .map(drop)
                                    .map_err(|e| e.to_string())
                            });
                        let _ = runtime.publish();
                        let _ = tx.send(result);
                    })
                    .expect("the film's extension mounts");
                self.mounting_surfaces
                    .push((id.to_owned(), hash.to_owned(), title, rx));
            }
        }
        let mut arrived = false;
        self.mounting_surfaces
            .retain(|(id, hash, title, rx)| match rx.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(()) => {
                            self.surfaces.insert(id.clone(), hash.clone());
                            if let Some(dock) = &self.dock {
                                let mut contributed = dock.contributed.borrow_mut();
                                contributed.retain(|(c, _)| c != id);
                                contributed.push((id.clone(), title.clone()));
                            }
                            arrived = true;
                        }
                        Err(error) => {
                            eprintln!("The film's extension {id} could not start: {error}")
                        }
                    }
                    false
                }
                Err(mpsc::TryRecvError::Empty) => true,
                Err(mpsc::TryRecvError::Disconnected) => false,
            });
        if arrived && let Some(editor) = &self.editor {
            editor.update(cx, |_, cx| cx.notify());
        }
        let Some(published) = self.kernel.published(PROJECT) else {
            return;
        };
        // The timeline's chrome (`workspace.chrome.timeline`, which
        // `timeline.chrome` writes).
        let chrome = published
            .workspace
            .get("chrome")
            .map(pm_model::json::to_canonical_string)
            .unwrap_or_default();
        if self.chrome.as_ref() != Some(&chrome) {
            if self.chrome.is_some()
                && let Some(timeline) = &self.timeline
            {
                let tokens = crate::builtins::timeline_chrome(
                    self.tokens.clone(),
                    &published.workspace,
                    &Value::Null,
                );
                timeline.update(cx, |t, cx| t.set_tokens(tokens, cx));
            }
            self.chrome = Some(chrome);
        }
        // The viewer: the project as the agent left it, drawn with what its
        // extensions define. Until either changes, the film's own document.
        let definitions = crate::extensions::native_effect_definitions(
            Some(&self.runtime),
            publication
                .contributions
                .as_deref()
                .map(crate::extensions::definitions)
                .unwrap_or_default(),
        );
        if definitions != self.resources.definitions {
            self.resources = Arc::new(pm_engine_service::resources::ResourceSnapshot {
                generation: self.resources.generation + 1,
                definitions,
                ..Default::default()
            });
        }
        let key = (published.revision(), self.resources.generation);
        let changed = match self.viewer {
            Some(at) => at != key,
            // The first look: nothing changed yet unless something did.
            None => self.resources.generation > 0 || self.seen_first.is_some_and(|r| r != key.0),
        };
        if self.seen_first.is_none() {
            self.seen_first = Some(key.0);
        }
        if changed {
            self.viewer = Some(key);
            self.pictures
                .0
                .borrow_mut()
                .set_viewer(&published, self.resources.clone());
        }
        // Running extensions hear the playhead once a beat (the film's
        // music: 100 BPM), not every frame.
        if !self.surfaces.is_empty() {
            let time = self.time.get();
            let beat = (time / 0.6).floor() as i64;
            if self.told_beat != Some(beat) {
                self.told_beat = Some(beat);
                self.runtime.update_facts(serde_json::json!({
                    "time": beat as f64 * 0.6 + 0.001, "playing": false,
                    "stage": [1920., 1080.], "light": false,
                }));
                let _ = self.runtime.publish();
            }
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
                    Rc::new(FilmTimeline {
                        time: Cell::new(0.),
                        runtime: self.runtime.clone(),
                    }),
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
                contributed: RefCell::default(),
            });
            let integration = dock.clone();
            let tokens = self.tokens.clone();
            let editor = cx.new(|cx| {
                let mut editor = Editor::new(tokens, None, cx);
                editor.integration = Some(integration);
                editor
            });
            // The agent's panel and layout tools act on this editor.
            if let Some(calls) = self.calls.take() {
                let target = editor.downgrade();
                cx.spawn(async move |cx| {
                    while let Ok(call) = calls.recv().await {
                        let target = target.clone();
                        let _ = cx.update(|cx| film_surface(call, target, cx));
                    }
                })
                .detach();
            }
            self.dock = Some(dock);
            self.timeline = Some(timeline);
            self.editor = Some(editor);
        }
        self.drain(cx);
        self.take_extensions(cx);
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

/// The film project's playhead at `t` into the film: it plays from when the
/// editor arrives, looping its 8 seconds, except through the light-leak
/// beat, where it slows to sit across the cut (4 s) the leak is set on, so
/// the transition plays as it lands.
fn film_time(t: f64) -> f64 {
    let sheet = pm_shell::stage::CueSheet::default();
    let start = sheet
        .start_of(pm_shell::stage::Scene::Beat(Change::GlassTimeline))
        .unwrap_or(0.);
    let leak = sheet
        .start_of(pm_shell::stage::Scene::Beat(Change::LightLeak))
        .unwrap_or(f64::INFINITY);
    let length = pm_shell::stage::beats::BEAT_SECONDS;
    let local = (t - start).max(0.);
    let into_leak = t - leak;
    let played = if into_leak <= 0. {
        local
    } else if into_leak < length {
        (leak - start) + into_leak * SLOW
    } else {
        local - length * (1. - SLOW)
    };
    played % 8.
}
/// How fast the project plays through the light-leak beat.
const SLOW: f64 = 0.15;

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
                        self.editor = Some(DemoEditor::new(
                            started,
                            self.tokens.clone(),
                            self.pictures.clone(),
                            self.time.clone(),
                        ))
                    }
                    Err(error) => eprintln!("The film's editor could not start: {error:#}"),
                }
            }
        }
        self.editor.as_mut()
    }
    /// Whether `change` is on screen: its tools answered and what it made
    /// runs (an extension mounted, the viewer drawing its transition).
    fn arrived(&self, change: Change) -> bool {
        use crate::film_script::{BEATS, GLASS, LEAK_TRANSITION};
        match self.applied.get(&change) {
            None => false,
            Some(Err(_)) => true,
            Some(Ok(())) => self.editor.as_ref().is_none_or(|e| match change {
                Change::GlassTimeline => e.mounted_extension(GLASS),
                Change::BeatMarkers => e.mounted_extension(BEATS),
                Change::LightLeak => e.draws_with(LEAK_TRANSITION),
                Change::ClassicLayout => true,
            }),
        }
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
    fn editor(&mut self, size: Size<Pixels>, window: &mut Window, cx: &mut App) -> AnyElement {
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
        editor.update(cx, |e, _| e.frame = Some(size));
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
    fn composer_size(&self) -> Size<Pixels> {
        size(px(560.), px(132.))
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
        self.arrived(change)
    }
    fn site(&self, _change: Change, area: Size<Pixels>) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), area)
    }
    fn at(&mut self, t: f64) {
        self.time.set(film_time(t));
    }
    fn settle(&mut self, cx: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        if let Some(editor) = self.started(true) {
            while !editor.mounted() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let _ = editor.editor(cx);
        }
        // Each change asked for, until it is on screen: the agent's panel
        // calls are answered here meanwhile (this thread is the editor's).
        loop {
            self.collect(false);
            if let Some(editor) = self.editor.as_mut() {
                editor.answer_calls(cx);
                let _ = editor.editor(cx);
            }
            let waiting = !self.asked.is_empty() || self.applied.keys().any(|c| !self.arrived(*c));
            if !waiting || std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::film_time;
    use pm_shell::stage::{Change, CueSheet, Scene, beats};
    #[test]
    fn the_films_playhead_runs_on_and_sits_across_the_cut_while_the_leak_lands() {
        let sheet = CueSheet::default();
        let leak = sheet.start_of(Scene::Beat(Change::LightLeak)).unwrap();
        // No jumps: the project plays on through the slowed beat.
        for edge in [leak, leak + beats::BEAT_SECONDS] {
            assert!((film_time(edge + 1e-3) - film_time(edge - 1e-3)).abs() < 0.01);
        }
        // From the wave to the beat's end the playhead is inside the 1.2 s
        // transition on the 4 s cut.
        for t in [leak + beats::LANDS, leak + beats::BEAT_SECONDS - 0.01] {
            let at = film_time(t);
            assert!((3.4..4.6).contains(&at), "{at}");
        }
    }
}
