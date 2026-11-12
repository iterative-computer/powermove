use crate::{panels, platform};
use gpui::{prelude::*, *};
use pm_engine_service::{EngineService, MediaCatalog, Viewer, ViewerConfig};
use pm_kernel::{
    session::*,
    state::{StateOp, StateTransaction},
    *,
};
use pm_protocol::{
    clock::{TransportCommand, TransportState},
    common::{PlaybackRate, TICKS_PER_SECOND, Ticks},
};
use pm_shell::{Command as ShellCommand, Host};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

#[derive(Clone)]
struct CompositionView {
    time: f64,
    timeline: pm_model::Value,
    viewer: pm_model::Value,
}

thread_local! {
    /// The time of an input the app dispatches itself (MCP `computer_use_panel`),
    /// which `event_time` reports instead of AppKit's last real event; `0`
    /// while no trace stamped it, so `event_time` reads the clock if asked.
    pub(crate) static DISPATCHING: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
}
pub struct Controller {
    pub builtins: crate::builtins::Host,
    pub extensions: crate::extensions::Extensions,
    pub font_broker: Option<pm_text::host::FontBroker>,
    /// Per-composition navigation view, scoped to the live document identity.
    composition_views: std::collections::BTreeMap<(u64, String), CompositionView>,
    pub kernel: KernelHandle,
    pub engine: Arc<EngineService>,
    pub host: Rc<RefCell<Host>>,
    pub profile: PathBuf,
    /// The original app's profile, when this Mac has one. A file it might hold
    /// opens as a copy while it runs. The native library is never affected.
    pub electron: Option<PathBuf>,
    /// Guarded folders whose originals the migration couldn't open
    /// (Settings › Media › Media in Protected Folders), read off the main thread.
    pub(crate) waiting_folders: Vec<pm_first_launch::WaitingFolder>,
    pub session: Option<SessionHandle>,
    pub published: Option<Arc<Published>>,
    pub mirror: Option<QueryMirror>,
    pub viewer: Option<Viewer>,
    pub media: MediaCatalog,
    pub(crate) media_refresh: crate::media_refresh::Refresh,
    /// What the media resolver found; `media` adds the renditions.
    pub(crate) media_base: MediaCatalog,
    /// A media resolution for this session has landed: only then do missing
    /// files read as offline (before it, everything is still loading).
    pub(crate) media_resolved: bool,
    /// Proxies, optimized media and their Background Tasks.
    pub pro_media: Arc<crate::pro_media::ProMedia>,
    /// Window ▸ Background Tasks, while open.
    pub tasks_window: Option<Entity<crate::tasks_window::TasksWindow>>,
    pub opening: bool,
    /// Held while a project opens and its panels settle: no App Nap or
    /// background throttling.
    pub(crate) busy: Option<crate::platform::Busy>,
    /// The project an open is reading, shown until it lands or fails, and
    /// then until the panels have drawn it (`settling`).
    pub opening_name: Option<String>,
    /// The project whose panels haven't published since it landed.
    pub settling: Option<String>,
    /// `PM_NATIVE_MAIN_TRACE` milestones of the current open.
    pub(crate) open_timer: crate::diag::OpenTimer,
    /// Project commands asked for while the project was arriving, bound to
    /// that open: they run once it has settled here (`settled`).
    pub(crate) after_open: crate::after_open::AfterOpen,
    /// The open whose project is settling.
    pub(crate) settling_open: u64,
    pub rows: crate::rows::LayerRows,
    /// The last refresh saw a save in flight (drives the status note).
    pub saving_note: bool,
    pub output_mode: crate::viewer2::OutputMode,
    /// View › Viewer Media for this editor's viewers.
    pub media_mode: pm_engine_service::MediaMode,
    /// Preview Resolution (viewer bar) for this editor's viewers.
    pub quality: pm_engine_service::Quality,
    /// The open project keeps HDR light.
    pub project_hdr: bool,
    pub scopes: crate::colour::Scopes,
    pub perf: crate::perf_panel::State,
    pub mixer: crate::mixer_panel::State,
    /// The workspace store this window last built (`crate::workspaces`).
    pub workspace_store: crate::workspaces::Cache,
    /// The Workspaces panel's last action, for its status line.
    pub workspaces_status: String,
    /// The layer a generated select's open menu edits, bound when it opened.
    pub(crate) generated_menu_layer: Option<String>,
    /// This window's shell view (its live layout).
    pub(crate) shell: Option<gpui::WeakEntity<pm_shell::view::Editor>>,
    /// Cloud media offered and downloading in this window.
    pub cloud: crate::cloud_media::State,
    pub captions: crate::captions_panel::State,
    pub shader: crate::shader_panel::State,
    pub takes: crate::takes_panel::State,
    pub notes: crate::notes_panel::State,
    pub safe_areas: bool,
    pub guides: bool,
    pub skimming: crate::skimming::Skimming,
    pub popout: Option<crate::viewer2::Popout>,
    /// The editor viewer's last configured output (size, display, headroom).
    pub main_output: Rc<std::cell::Cell<Option<pm_engine_service::Output>>>,
    pub selected: Option<String>,
    pub message: String,
    pub path: Option<PathBuf>,
    pub recent: Vec<(String, String)>,
    pub library: Option<Entity<crate::project_library::ProjectLibrary>>,
    /// Home is asked for over this window's project (toolbar, File ▸ Projects…).
    pub home: bool,
    /// Home was drawn last frame (it refreshes and takes focus as it appears).
    pub(crate) home_visible: bool,
    /// A restored window is checking whether its project reopens.
    pub(crate) reopening: bool,
    /// Wakes the playback loop, which places the playhead layers.
    pub(crate) playback_wake: Option<async_channel::Sender<()>>,
    pub errors: crate::error_log::Shared,
    pub rate: pm_time::Rate,
    pub duration: f64,
    pub dimensions: [f64; 2],
    /// Composition background (`bg`); `None` when transparent.
    pub background: Option<u32>,
    /// Keyframes in the open composition, for the status bar.
    pub keyframes: usize,
    pub recovery_pending: bool,
    /// The document whose stop (an internal error) this window already
    /// offered to reopen.
    pub stop_offered: Option<u64>,
    /// The next open is Reopen after such a stop (its recovery prompt says so).
    reopening_stopped: bool,
    pub mcp: Arc<crate::mcp::Host>,
    /// Every open document's override answers, for agents' jobs.
    pub job_overrides: crate::raster_overrides::OverrideWindows,
    /// The project and revision Home's poster was last written from.
    pub poster_revision: Option<((String, u64), std::time::Instant)>,
    /// This window's slot in `mcp`: the project it shows is its own.
    pub window_key: u64,
    pub close_pending: bool,
    /// A change could not be written to the recovery data and no save has
    /// succeeded since: said in a sheet once, and in the subtitle until then.
    unprotected: bool,
    /// What this window's close (or the app's quit) is waiting for, once
    /// the person has answered: shown in the status line over any other
    /// message, and named by a second Quit's prompt.
    pub closing_note: Option<String>,
    pub close_window: Option<gpui::AnyWindowHandle>,
    pub close_operation: Option<crate::mcp::Operation>,
    pub gizmo_lines: Vec<pm_engine_service::OverlayLine>,
    /// The toolbar tool, mirrored from shell commands for native surfaces.
    pub tool: String,
    /// The Shape tool's variant (`rect`, `rounded`, `ellipse`, `polygon`, `star`).
    pub shape_variant: String,
    pub view: crate::gizmo::ViewPlacement,
    /// The pending or running write of `view`.
    pub(crate) view_write: Option<gpui::Task<()>>,
    pub clipboard: crate::layer_commands::Clipboard,
    /// Commands that cannot run now; menus and the palette show them off.
    pub disabled: std::collections::BTreeSet<String>,
    pub save_pending: bool,
    pub close_after_save: Option<crate::lifecycle::Intent>,
    /// Queued batches and the cancel flag of the running one.
    pub(crate) imports: std::collections::VecDeque<(
        pm_builtins::media_import::ImportRequest,
        Option<crate::media_import::ExtensionImport>,
    )>,
    pub(crate) importing: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Arrow-key nudges less than [`NUDGE_IDLE`] apart are one undo step
    /// (keymap.ts): the history mark taken before the first, and the timer
    /// that squashes the burst once the keys rest.
    nudge_mark: Option<ActorHistoryMark>,
    nudge_idle: Option<gpui::Task<()>>,
}
/// How long arrow keys rest before a run of nudges becomes its undo step.
const NUDGE_IDLE: std::time::Duration = std::time::Duration::from_millis(180);

/// Why a project opened as a copy, in user language (never the enum name):
/// said when it opens. The window's subtitle keeps saying it
/// ([`copy_subtitle`]).
pub fn copy_notice(decision: &pm_kernel::locks::OpenDecision) -> Option<&'static str> {
    use pm_kernel::locks::{CopyReason::*, OpenDecision};
    let OpenDecision::AsCopy(reason) = decision else {
        return None;
    };
    Some(match reason {
        ForeignLibrary => "Opened as a copy because the project belongs to another library.",
        ChangedOnDisk => "Opened as a copy because the file changed on disk.",
        OfflineFile => "Opened as a copy because the file isn’t downloaded.",
        ReadOnly => {
            "This is a copy: the file is read-only, so changes are kept in your projects. Use Save As to write a file."
        }
        OlderElectron => {
            "This is a copy: the original Powermove app is running, so changes are kept in your projects. Use Save As to write a file."
        }
        Locked => {
            "This is a copy: the file is open somewhere else, so changes are kept in your projects. Use Save As to write a file."
        }
        UncertainLock => {
            "This is a copy: Powermove couldn’t confirm the file isn’t open elsewhere, so changes are kept in your projects. Use Save As to write a file."
        }
    })
}

/// The window subtitle of a project: its save state, and for a copy that
/// stands in for a file (the entry's `copyOf`), which file and why, for as
/// long as the window shows it.
pub fn copy_subtitle(session: Option<&pm_model::Value>, state: &'static str) -> String {
    let copy = session.and_then(|meta| meta.get("copyOf"));
    let Some(file) = copy
        .and_then(|c| c.get("path"))
        .and_then(pm_model::Value::as_str)
        .and_then(|path| std::path::Path::new(path).file_name())
    else {
        return state.to_owned();
    };
    let why = match copy
        .and_then(|c| c.get("reason"))
        .and_then(pm_model::Value::as_str)
    {
        Some("ReadOnly") => "the file is read-only",
        Some("OlderElectron") => "the original app is running",
        Some("Locked") => "the file is open elsewhere",
        _ => "the file couldn’t be locked",
    };
    let copy = format!("Copy of {} ({why})", file.to_string_lossy());
    if state.is_empty() {
        copy
    } else {
        format!("{copy} — {state}")
    }
}

/// The offer when a file that opens in place has copies in the library that
/// hold work for it (earlier builds opened files in syncing folders as
/// copies named "… — Native"). `true` opens the copy.
fn stranded_copies_alert(file: &str, copies: &[String]) -> crate::alert::Alert<bool> {
    use crate::alert::{Alert, Button, Key};
    let (message, which) = match copies {
        [one] => (
            format!("Your projects hold a copy of “{file}” with work that isn’t in this file."),
            format!("“{one}”"),
        ),
        [first, rest @ ..] => (
            format!("Your projects hold copies of “{file}” with work that isn’t in this file."),
            format!("“{first}” and {} more", rest.len()),
        ),
        [] => (String::new(), String::new()),
    };
    Alert::new(
        message,
        format!(
            "Powermove opened this file as a copy before and kept your changes there instead of in the file: {which}. The file now opens and saves in place. The copy stays in your projects either way."
        ),
        vec![
            Button::new("Open the Copy", true).default(),
            Button::new("Keep Working in This File", false).key(Key::Escape),
        ],
    )
}
/// Said once when a file in a folder a sync client keeps opens in place.
const SYNCED_FOLDER: &str = "In a synced folder: a change made on another computer is noticed when you save, and both versions are kept.";

/// Whether a background recovery-data failure for `project` leaves the work
/// in the window showing `shown` unprotected.
fn unprotects(project: &str, shown: Option<&str>) -> bool {
    shown == Some(project)
}

/// Said when a file opened as a new copy although the library holds a copy
/// made under its name: that copy was made of another file (this one
/// replaced it, or changed somewhere else since), so it was not opened in
/// its place. `true` opens the earlier copy.
fn earlier_copy_alert(file: &str, copies: &[String]) -> crate::alert::Alert<bool> {
    use crate::alert::{Alert, Button};
    let which = match copies {
        [one] => format!("“{one}”"),
        [first, rest @ ..] => format!("“{first}” and {} more", rest.len()),
        [] => String::new(),
    };
    Alert::new(
        format!("“{file}” isn’t the file your earlier copy was made from."),
        format!(
            "A file with this name opened as a copy before, and your changes to it are in your projects: {which}. The file here now is a different one (it was replaced, or changed somewhere else), so it opened as a new copy of its own. The earlier copy stays in your projects either way."
        ),
        vec![
            Button::new("Keep This One", false).default().cancel(),
            Button::new("Open the Earlier Copy", true),
        ],
    )
}

pub struct Bridge(pub Entity<Controller>);
impl pm_shell::integration::Integration for Bridge {
    fn observe(&self, cx: &mut gpui::Context<pm_shell::view::Editor>) {
        cx.observe(&self.0, |_, _, cx| cx.notify()).detach();
        let shell = cx.weak_entity();
        self.0.update(cx, |v, _| v.shell = Some(shell));
    }
    fn command(&self, cmd: &ShellCommand, w: &mut Window, cx: &mut App) -> bool {
        self.0.update(cx, |v, cx| v.command(cmd, w, cx))
    }
    fn disabled(&self, cx: &App) -> std::collections::BTreeSet<String> {
        let v = self.0.read(cx);
        let mut off = v.disabled.clone();
        // While a project arrives its commands stay available: they wait for
        // it (`Controller::command`) instead of being refused.
        if v.opening || v.settling.is_some() {
            off.retain(|name| !crate::layer_commands::waits_for_project(name));
        }
        off
    }
    fn panel(
        &self,
        id: &str,
        bounds: pm_shell::layout::Rect,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.0.update(cx, |v, cx| {
            panels::opening(v, id, bounds)
                .or_else(|| v.extension_panel(id, bounds, cx))
                .or_else(|| panels::scopes(v, id, cx))
                .or_else(|| crate::perf_panel::panel(v, id, cx))
                .or_else(|| crate::mixer_panel::panel(v, id, bounds, cx))
                .or_else(|| crate::captions_panel::panel(v, id, bounds, w, cx))
                .or_else(|| crate::shader_panel::panel(v, id, bounds, w, cx))
                .or_else(|| crate::takes_panel::panel(v, id, bounds, cx))
                .or_else(|| crate::notes_panel::panel(v, id, bounds, w, cx))
                .or_else(|| crate::workspaces::panel(v, id, bounds, cx))
                .or_else(|| crate::generated_panel::panel(v, id, bounds, cx))
                .or_else(|| v.builtin_panel(id, bounds, w, cx))
        })
    }
    fn home(&self, w: &mut Window, cx: &mut App) -> Option<AnyElement> {
        self.0.update(cx, |v, cx| v.home_element(w, cx))
    }
    fn paints_own_ground(&self, id: &str, cx: &App) -> bool {
        id == "timeline" && !self.0.read(cx).extensions.owns_panel(id)
    }
    fn native_panel(&self, id: &str, cx: &App) -> bool {
        self.0.read(cx).extensions.owns_panel(id)
    }
    fn workspaces(&self, cx: &App) -> Option<std::rc::Rc<pm_shell::workspace::Store>> {
        Some(self.0.read(cx).workspace_store())
    }
    fn note(&self, text: &str, cx: &mut App) {
        let text = text.to_owned();
        self.0.update(cx, |v, cx| v.note(text, cx));
    }
    fn write_workspace(&self, write: pm_shell::workspace::Write, cx: &mut App) {
        self.0.read(cx).write_workspace(write);
    }
    fn contributed_panels(&self, cx: &App) -> Vec<(String, String)> {
        self.0.read(cx).extensions.contributed.clone()
    }
    fn open_panels(&self, open: &[&str], cx: &mut App) {
        self.0.update(cx, |v, cx| {
            // Each layout pass places the timeline again, if it draws it.
            v.builtins.timeline_place = None;
            v.sync_extension_panels(open);
            v.sync_parking(open, cx);
        });
    }
    fn shown_panels(&self, shown: &[&str], cx: &mut App) {
        self.0.update(cx, |v, _| v.hide_extension_panels(shown));
    }
    fn panel_menu(
        &self,
        id: &str,
        entries: Vec<pm_shell::MenuEntry>,
        at: (f32, f32),
        cx: &mut App,
    ) -> Option<Vec<pm_shell::MenuEntry>> {
        self.0
            .update(cx, |v, cx| v.extension_panel_menu(id, entries, at, cx))
    }
    fn header(
        &self,
        id: &str,
        bounds: pm_shell::layout::Rect,
        w: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.0.update(cx, |v, cx| {
            if id == "scopes" {
                return Some(v.scopes_header(cx));
            }
            v.builtin_header(id, bounds, w, cx)
        })
    }
    fn project_id(&self, cx: &App) -> Option<String> {
        self.0.read(cx).session.as_ref().map(|s| s.project.clone())
    }
    fn status(&self, cx: &App) -> String {
        self.0.read(cx).status_text()
    }
    fn status_trailing(&self, cx: &App) -> String {
        let c = self.0.read(cx);
        if c.published.is_none() {
            return String::new();
        }
        panels::format_facts(c.dimensions, c.rate.to_f64())
    }
    fn status_alert(&self, cx: &App) -> String {
        panels::offline_summary(self.0.read(cx).offline_count())
    }
    fn status_alert_pressed(&self, _window: &mut Window, cx: &mut App) {
        self.0.update(cx, |v, cx| v.show_offline_media(cx));
    }
    fn drop_paths(&self, paths: Vec<PathBuf>, cx: &mut App) {
        self.0.update(cx, |v, cx| v.import_dropped(paths, cx));
    }
}
/// The size the editor lays a composition out at: its stored size, held to
/// at least one pixel a side. A file needs only finite numbers to open, so a
/// stored 0 (or a negative) is possible; fit, zoom and overlay math divide
/// by this. The stored value is not rewritten: it changes only when the
/// person edits the composition's settings.
fn composition_size(w: f64, h: f64) -> [f64; 2] {
    let side = |v: f64| if v.is_finite() { v.max(1.) } else { 1. };
    [side(w), side(h)]
}
/// Where the playhead of a composition being left is: the viewer's clock, or,
/// while a viewer bound a moment ago has not reported one yet, the time last
/// asked for (written to the transport state by every seek). Reading only the
/// clock left a composition at 0 s when the person came back to it and left
/// again before its first frame.
fn left_at(clock: Option<f64>, asked: Option<f64>) -> f64 {
    clock.or(asked).unwrap_or(0.)
}
impl Controller {
    /// A passing status note (v1's status toast): shown in the status line,
    /// then the line returns to Saved/Edited unless something replaced it.
    pub fn note(&mut self, text: String, cx: &mut Context<Self>) {
        let hold = self.host.borrow().tokens.motion.note_hold;
        self.message = text.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs_f64(hold))
                .await;
            let _ = this.update(cx, |v, cx| {
                if v.message == text {
                    v.message.clear();
                    cx.notify();
                }
            });
        })
        .detach();
    }
    /// Status bar copy (language §14): a pending message or error, else
    /// `Saved`/`Edited` with layer and keyframe counts.
    pub fn status_text(&self) -> String {
        if let Some(note) = &self.closing_note {
            return note.clone();
        }
        // "Saved" is the state word the computed line already starts with.
        if (!self.message.is_empty() && self.message != "Saved") || self.session.is_none() {
            return self.message.clone();
        }
        let dirty = self.published.as_ref().is_some_and(|p| p.dirty);
        panels::status_line(
            dirty,
            self.path.is_some(),
            self.layers().len(),
            self.keyframes,
        )
    }
    pub fn new(
        kernel: KernelHandle,
        engine: Arc<EngineService>,
        profile: PathBuf,
        electron: Option<PathBuf>,
        mcp: Arc<crate::mcp::Host>,
        host: Rc<RefCell<Host>>,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::diag::EDITORS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let pro_media = Arc::new(crate::pro_media::ProMedia::new(
            profile.clone(),
            crate::media::objects_dir(&profile),
            engine.export_activity().flag(),
        ));
        pro_media.refresh_usage();
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |v, cx| v.refresh_waiting_folders(cx));
        })
        .detach();
        cx.observe_global::<crate::pro_media::MediaPassDone>(|v, cx| v.refresh_waiting_folders(cx))
            .detach();
        crate::render_frames::install(engine.render_cache());
        sync_render_storage(&pro_media, &engine);
        engine
            .render_cache()
            .set_background(pro_media.preferences().background());
        let pro_events = pro_media.event_receiver.clone();
        cx.spawn(async move |this, cx| {
            while let Ok(first) = pro_events.recv().await {
                let mut renditions = matches!(first, crate::pro_media::Event::Renditions);
                while let Ok(e) = pro_events.try_recv() {
                    renditions |= matches!(e, crate::pro_media::Event::Renditions);
                }
                if this
                    .update(cx, |v, cx| v.pro_media_changed(renditions, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let events = kernel.subscribe();
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            while let Ok(e) = events.recv() {
                if tx.send_blocking(e).is_err() {
                    break;
                }
            }
        });
        cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                // Coalesce a burst (an MCP run publishes several revisions):
                // one refresh reads the newest publication for all of them.
                let mut events = vec![first];
                while let Ok(e) = rx.try_recv() {
                    events.push(e);
                }
                if this
                    .update(cx, |v, cx| {
                        for e in events {
                            match e {
                                // A change acknowledged earlier could not be made
                                // durable afterwards: the window showing that
                                // project says so exactly as for a change
                                // refused at the time (sheet, subtitle until a
                                // save succeeds, Error Log row).
                                KernelEvent::Durability { project, message } => {
                                    let shown = v.session.as_ref().map(|s| s.project.as_str());
                                    if unprotects(&project, shown) {
                                        v.notice(pm_kernel::Notice::NotDurable { message }, cx);
                                    }
                                }
                                KernelEvent::Fault { message, .. } => {
                                    v.message = "Something went wrong; see the Error Log.".into();
                                    v.errors.borrow_mut().push("Kernel", &message);
                                }
                                KernelEvent::Notice { project, notice }
                                    if v.session.as_ref().is_some_and(|s| s.project == project) =>
                                {
                                    v.notice(notice, cx);
                                }
                                _ => {}
                            }
                        }
                        v.refresh(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        for (chord, command) in [
            ("j", "shuttleJ"),
            ("k", "shuttleK"),
            ("l", "shuttleL"),
            ("ctrl+j", "shuttleReverse"),
        ] {
            host.borrow_mut()
                .add_bindings([pm_shell::keyboard::Binding {
                    id: format!("app.{command}"),
                    chord: chord.into(),
                    command: command.into(),
                    args: vec![],
                    priority: 200,
                    in_fields: false,
                    repeat: false,
                    loose: false,
                }]);
        }
        let layer = host.borrow().viewer_layer();
        pm_engine_service::layers::configure_layer(
            objc2::MainThreadMarker::new().unwrap(),
            &layer,
            &engine.metal_device(),
        )
        .expect("viewer layer");
        host.borrow().set_viewer_label("Composition viewer");
        let mut builtins = crate::builtins::Host::default();
        builtins.export_tasks = crate::export_coordinator::tasks();
        Self {
            builtins,
            extensions: Default::default(),
            font_broker: None,
            composition_views: Default::default(),
            kernel,
            engine,
            profile,
            electron,
            waiting_folders: vec![],
            host,
            session: None,
            published: None,
            mirror: None,
            viewer: None,
            media: MediaCatalog::default(),
            media_refresh: Default::default(),
            media_base: MediaCatalog::default(),
            media_resolved: false,
            pro_media,
            tasks_window: None,
            opening: false,
            busy: None,
            opening_name: None,
            settling: None,
            open_timer: Default::default(),
            after_open: Default::default(),
            settling_open: 0,
            rows: Default::default(),
            saving_note: false,
            output_mode: crate::viewer2::OutputMode::from_env(),
            media_mode: Default::default(),
            quality: Default::default(),
            project_hdr: false,
            scopes: Default::default(),
            perf: Default::default(),
            mixer: Default::default(),
            workspace_store: Default::default(),
            workspaces_status: String::new(),
            generated_menu_layer: None,
            shell: None,
            cloud: Default::default(),
            captions: Default::default(),
            shader: Default::default(),
            takes: Default::default(),
            notes: Default::default(),
            safe_areas: false,
            guides: false,
            skimming: Default::default(),
            popout: None,
            main_output: Default::default(),
            selected: None,
            // The status bar holds facts, not instructions (review shell 2 #11).
            message: "No project".into(),
            path: None,
            recent: vec![],
            library: None,
            home: false,
            home_visible: false,
            reopening: false,
            playback_wake: None,
            errors: crate::error_log::shared(cx),
            rate: pm_time::Rate::FPS_30,
            duration: 1.,
            dimensions: [1920., 1080.],
            background: None,
            keyframes: 0,
            recovery_pending: false,
            stop_offered: None,
            reopening_stopped: false,
            mcp,
            job_overrides: Default::default(),
            poster_revision: None,
            window_key: 0,
            close_pending: false,
            unprotected: false,
            closing_note: None,
            close_window: None,
            close_operation: None,
            gizmo_lines: vec![],
            tool: "toolSelect".into(),
            shape_variant: "rect".into(),
            view: Default::default(),
            view_write: None,
            clipboard: Default::default(),
            disabled: crate::layer_commands::disabled(None, &[], &Default::default()),
            save_pending: false,
            close_after_save: None,
            imports: Default::default(),
            importing: None,
            nudge_mark: None,
            nudge_idle: None,
        }
    }
    pub fn initialize(&mut self, opening: Option<crate::entry::Pending>, cx: &mut Context<Self>) {
        self.load_recent(cx);
        let cache = crate::media::cache_root(&self.profile);
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = crate::media::sweep_cache(&cache) {
                    eprintln!("Media cache cleanup: {error}");
                }
            })
            .detach();
        match opening {
            Some(crate::entry::Pending::File(path)) => self.open_file(path, cx),
            Some(crate::entry::Pending::Library(project)) => self.open_named(
                SessionSource::Library {
                    profile: self.profile.clone(),
                    project,
                },
                None,
                cx,
            ),
            Some(crate::entry::Pending::Session(session)) => {
                // Gone before this window took it: nothing to show.
                if self.kernel.published(&session.project).is_some() {
                    self.adopt(session, pm_kernel::locks::OpenDecision::Writable, false, cx);
                    self.mcp.activate_window(self.window_key);
                }
            }
            Some(crate::entry::Pending::Adopt { reply, name, file }) => {
                self.adopt_reply(*reply, name, file, cx)
            }
            None => self.reopen_restored(cx),
        }
    }
    /// A window restored at launch reopens its project when that project has
    /// unsaved work from a crash (so the person is asked about it at once),
    /// when the system keeps documents open across quits, or when Settings ›
    /// General › Reopen Last Project on Launch is on (v1's `restoreWindows`,
    /// on unless turned off).
    fn reopen_restored(&mut self, cx: &mut Context<Self>) {
        let Some((project, keep)) = self.host.borrow().restored_project.clone() else {
            return;
        };
        let keep = keep
            || self
                .kernel
                .state()
                .settings
                .get("restoreWindows")
                .and_then(pm_model::Value::as_bool)
                != Some(false);
        let profile = self.profile.clone();
        let found = cx.background_executor().spawn(async move {
            let unsaved =
                !pm_kernel::journal::sessions(&profile.join("journals").join(&project)).is_empty();
            if !(unsaved || keep) {
                return None;
            }
            let row = pm_kernel::library::LibraryStore::open(&profile)
                .projects()
                .ok()?
                .into_iter()
                .find(|row| row.id == project)?;
            Some((row.id, row.name))
        });
        // Home waits for the answer, so a project that reopens never
        // flashes Home first.
        self.reopening = true;
        cx.spawn(async move |this, cx| {
            let found = found.await;
            let _ = this.update(cx, |v, cx| {
                v.reopening = false;
                cx.notify();
            });
            if let Some((id, name)) = found {
                let _ = this.update(cx, |v, cx| {
                    if v.session.is_none() && !v.opening {
                        v.open_named(
                            SessionSource::Library {
                                profile: v.profile.clone(),
                                project: id,
                            },
                            Some(name),
                            cx,
                        );
                    }
                });
            }
        })
        .detach();
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let _span = crate::diag::span("refresh");
        let Some(session) = &self.session else {
            cx.notify();
            return;
        };
        let Some(mut p) = self.kernel.published(&session.project) else {
            return;
        };
        let switched = self.published.as_ref().is_some_and(|old| {
            old.snapshot.doc_id == p.snapshot.doc_id
                && old.snapshot.open_comp() != p.snapshot.open_comp()
        });
        if switched {
            self.host.borrow_mut().close_flowchart();
            if let Some(old) = &self.published
                && let Some(id) = old.snapshot.open_comp()
            {
                self.composition_views.insert(
                    (old.snapshot.doc_id, id),
                    CompositionView {
                        time: left_at(
                            self.viewer
                                .as_ref()
                                .and_then(|v| v.status().clock)
                                .map(|c| c.media_time.get() as f64 / TICKS_PER_SECOND as f64),
                            old.ui_state.transport.get("time").and_then(|v| v.as_f64()),
                        ),
                        timeline: old.ui_state.timeline.clone(),
                        viewer: old.ui_state.viewer.clone(),
                    },
                );
            }
            let key = (
                p.snapshot.doc_id,
                p.snapshot.open_comp().unwrap_or_default(),
            );
            let saved = self.composition_views.get(&key).cloned();
            let mut next = (*p).clone();
            let time = if let Some(saved) = saved {
                next.ui_state.timeline = saved.timeline;
                next.ui_state.viewer = saved.viewer;
                saved.time
            } else {
                0.
            };
            self.view.written();
            self.view_write = None;
            if let Ok(chrome) = pm_panel_viewerchrome::Chrome::read(&next.ui_state) {
                self.view.adopt(&chrome.navigation);
            }
            p = Arc::new(next);
            self.published = Some(p.clone());
            self.bind_composition();
            self.apply_viewer_options();
            self.rebuild_popout_viewer();
            self.seek(time);
        }
        self.refresh_media(cx);
        // Raster override answers are for one revision; frames ask afresh.
        crate::raster_overrides::published(&self.engine, &p);
        if let Some(viewer) = &self.viewer {
            let _span = crate::diag::span("refresh.publish");
            if let Err(e) = viewer.publish(&p) {
                self.message = e.to_string();
            }
        }
        if let Some(popout) = &self.popout
            && let Some(viewer) = &*popout.viewer.borrow()
            && let Err(e) = viewer.publish(&p)
        {
            self.message = format!("Popout viewer: {e}");
        }
        {
            let _span = crate::diag::span("refresh.mirror");
            if let Some(m) = &mut self.mirror {
                m.sync(&p);
            } else {
                self.mirror = Some(QueryMirror::new(&p));
            }
        }
        self.sync_rows(&p, cx);
        if p.fault.is_some() && self.stop_offered != Some(p.snapshot.doc_id) {
            self.stop_offered = Some(p.snapshot.doc_id);
            self.offer_reopen(&p, cx);
        }
        self.follow_project_hdr();
        let saving = p.saving || self.save_pending;
        if let Some(note) = save_note(self.saving_note, saving, p.dirty, &self.message) {
            self.message = note.into();
        }
        self.saving_note = saving;
        self.selected = p.selection.primary().map(str::to_owned);
        if let Ok(chrome) = pm_panel_viewerchrome::Chrome::read(&p.ui_state) {
            self.view.adopt(&chrome.navigation);
            // Preview Resolution in the viewer bar: the engine renders at it.
            let quality = crate::viewer2::engine_quality(chrome.quality);
            if self.quality != quality {
                self.quality = quality;
                self.apply_viewer_options();
            }
        }
        self.path = p.session.as_ref().and_then(|s| match &s.destination {
            SessionDestination::File(path) => Some(path.clone()),
            _ => None,
        });
        let title = p
            .snapshot
            .document
            .root()
            .and_then(|r| r.get("name"))
            .and_then(crate::node_string)
            .unwrap_or("Untitled");
        self.host.borrow().set_document_metadata(
            title,
            &copy_subtitle(
                p.session.as_ref().map(|s| &s.metadata),
                if self.unprotected && !saving {
                    "Not protected: save now"
                } else {
                    pm_shell::document_subtitle(p.dirty, saving)
                },
            ),
            p.dirty,
            self.path.as_deref(),
        );
        platform::history_labels(
            p.history
                .undo_label
                .as_deref()
                .filter(|_| p.history.can_undo),
            p.history
                .redo_label
                .as_deref()
                .filter(|_| p.history.can_redo),
        );
        self.published = Some(p);
        self.update_builtins(cx);
        if let Some(timeline) = &self.builtins.timeline {
            timeline.update(cx, |t, cx| t.published(cx));
        }
        if let Some(gizmos) = &self.builtins.gizmos {
            gizmos.update(cx, |g, cx| g.refresh(cx));
        }
        self.update_disabled();
        cx.notify();
    }
    pub fn layers(&self) -> &[LayerInfo] {
        &self.rows.rows
    }
    /// Rows follow the newest publication; large lists are built on a worker.
    fn sync_rows(&mut self, p: &Published, cx: &mut Context<Self>) {
        use crate::rows::{self, Plan};
        let key = rows::key(&p.snapshot);
        match self.rows.plan(key, rows::layer_count(&p.snapshot)) {
            Plan::Current => {}
            Plan::Inline => {
                let _span = crate::diag::span("refresh.rows");
                if self.rows.accept(key, rows::list(&p.snapshot), Some(key)) {
                    // O(keyframes): a baked project holds hundreds of thousands.
                    let snapshot = p.snapshot.clone();
                    cx.spawn(async move |this, cx| {
                        let count = cx
                            .background_executor()
                            .spawn(async move { rows::keyframe_count(&snapshot) })
                            .await;
                        let _ = this.update(cx, |v, cx| {
                            if v.published.as_ref().map(|p| rows::key(&p.snapshot)) == Some(key) {
                                v.keyframes = count;
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
            }
            Plan::Worker => {
                let snapshot = p.snapshot.clone();
                cx.spawn(async move |this, cx| {
                    let list = cx
                        .background_executor()
                        .spawn(
                            async move { (rows::list(&snapshot), rows::keyframe_count(&snapshot)) },
                        )
                        .await;
                    let _ = this.update(cx, |v, cx| {
                        let latest = v.published.as_ref().map(|p| rows::key(&p.snapshot));
                        let (list, keyframes) = list;
                        if v.rows.finish(key, list, latest) {
                            v.keyframes = keyframes;
                            cx.notify();
                        }
                        if let Some(p) = v.published.clone() {
                            v.sync_rows(&p, cx);
                        }
                    });
                })
                .detach();
            }
        }
    }
    pub fn call(&mut self, call: Call, cx: &mut Context<Self>) {
        let input = pm_engine_service::trace::enabled().then(Self::event_time);
        self.call_at(call, input, cx);
    }
    /// `input`: the edit's input time for the viewer trace, if traced.
    pub fn call_at(&mut self, call: Call, input: Option<f64>, cx: &mut Context<Self>) {
        if self.close_pending {
            return;
        }
        let Some(session) = &self.session else { return };
        if let Some(input) = input
            && matches!(
                call,
                Call::Transact { .. } | Call::Gesture(GestureCall::Step { .. })
            )
            && let Some(viewer) = &self.viewer
        {
            viewer.input_event(input);
        }
        let pending = self
            .kernel
            .call(&session.project, Request::new(Caller::native(), call));
        cx.spawn(async move |this, cx| {
            let result = pending.await;
            let _ = this.update(cx, |v, cx| {
                match result {
                    Some(Ok(ack)) => {
                        if let Reply::Throw(e) = ack.reply {
                            v.message = e.to_string();
                        }
                    }
                    other => v.edit_failed(other.and_then(Result::err)),
                }
                v.refresh(cx);
            });
        })
        .detach();
    }
    /// Plans off the main thread: nudging reads layer matrices. Whether a plan
    /// exists is known only there, after the key is spent, so an empty one says so.
    fn run_plan(&mut self, name: &str, args: &[serde_json::Value], cx: &mut Context<Self>) -> bool {
        let Some(published) = self.published.clone() else {
            return false;
        };
        if name == "nudgeSelection" {
            self.extend_nudge_burst(cx);
        }
        let (name, args, time) = (name.to_owned(), args.to_vec(), self.time());
        cx.spawn(async move |this, cx| {
            let plan = cx
                .background_executor()
                .spawn(async move {
                    crate::layer_commands::command_plan(&name, &args, &published, time)
                })
                .await;
            let _ = this.update(cx, |v, cx| match plan {
                Some(plan) => v.run_layer_plan(plan, cx),
                None => {
                    v.message = "Nothing to change".into();
                    cx.notify();
                }
            });
        })
        .detach();
        true
    }
    /// The first nudge of a burst marks history before its own write reaches
    /// the kernel (calls run in order); every nudge restarts the rest timer.
    fn extend_nudge_burst(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.session else { return };
        if self.nudge_idle.is_none() {
            self.nudge_mark = None;
            let mark = self.kernel.call(
                &session.project,
                Request::new(Caller::native(), HistoryCall::Mark),
            );
            cx.spawn(async move |this, cx| {
                if let Some(Ok(Ack {
                    reply: Reply::Mark(mark),
                    ..
                })) = mark.await
                {
                    let _ = this.update(cx, |v, _| v.nudge_mark = Some(mark));
                }
            })
            .detach();
        }
        self.nudge_idle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NUDGE_IDLE).await;
            let _ = this.update(cx, |v, cx| v.close_nudge_burst(cx));
        }));
    }
    /// Squashes the burst into one "Nudge Selection" step. The kernel refuses
    /// when anything else wrote since the mark; the nudges then stay separate.
    fn close_nudge_burst(&mut self, cx: &mut Context<Self>) {
        self.nudge_idle = None;
        let (Some(mark), Some(session)) = (self.nudge_mark.take(), &self.session) else {
            return;
        };
        let (kernel, project) = (self.kernel.clone(), session.project.clone());
        cx.background_spawn(async move {
            let Some(published) = kernel.published(&project) else {
                return;
            };
            let squash = HistoryCall::Squash {
                mark,
                label: "Nudge Selection".into(),
                expected_revision: published.revision(),
            };
            let _ = kernel
                .call(&project, Request::new(Caller::native(), squash))
                .await;
        })
        .detach();
    }
    /// Selected keyframes take precedence over their layers, as Delete does.
    fn copy_selection(&mut self, cut: bool, cx: &mut Context<Self>) -> bool {
        use crate::layer_commands::Clipboard;
        let Some(published) = self.published.clone() else {
            return false;
        };
        let keys = self
            .builtins
            .timeline
            .as_ref()
            .map(|t| t.read(cx).copied_keys())
            .unwrap_or_default();
        let (clipboard, text) = if !keys.is_empty() {
            (Clipboard::Keys(keys), "Powermove keyframes".to_owned())
        } else {
            let selected = published.selection.layers();
            let root = published.snapshot.root();
            let layers: Vec<pm_model::Value> = selected
                .iter()
                .filter_map(|id| {
                    let root = root?;
                    let at = pm_kernel::model::find_layer(root, id)?;
                    Some(pm_kernel::model::layer_at(root, &at)?.to_value())
                })
                .collect();
            if layers.is_empty() {
                return false;
            }
            let names: Vec<&str> = layers
                .iter()
                .filter_map(|l| l.get("name").and_then(pm_model::Value::as_str))
                .collect();
            let text = names.join("\n");
            (Clipboard::Layers(layers), text)
        };
        self.clipboard = clipboard;
        // Replaces any older system media, as a text copy does.
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.update_disabled();
        if cut && let Some(timeline) = self.builtins.timeline.clone() {
            let time = self.time();
            timeline.update(cx, |t, cx| t.command("delete", time, None, false, cx));
        }
        true
    }
    fn paste_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        use crate::layer_commands::Clipboard;
        // Figma's layers claim the paste ahead of the app's own clipboard.
        if self.paste_figma(cx) {
            return true;
        }
        let time = self.time();
        match self.clipboard.clone() {
            Clipboard::Empty => false,
            Clipboard::Keys(tracks) => {
                let Some(timeline) = self.builtins.timeline.clone() else {
                    return false;
                };
                timeline.update(cx, |t, cx| t.paste_keys(tracks, time, cx));
                true
            }
            Clipboard::Layers(layers) => {
                self.run_layer_plan(crate::layer_commands::paste_plan(layers, time), cx);
                true
            }
        }
    }
    /// Menus, palette and toolbar enablement for the current document state.
    pub fn update_disabled(&mut self) {
        self.disabled = crate::layer_commands::disabled(
            self.published.as_deref(),
            self.layers(),
            &self.clipboard,
        );
    }
    /// One Undo step, then the selection it leads to.
    pub(crate) fn run_layer_plan(
        &mut self,
        plan: crate::layer_commands::Plan,
        cx: &mut Context<Self>,
    ) {
        let revision = self.published.as_ref().map(|p| p.revision());
        let Some(call) = crate::layer_commands::transaction(&plan, revision) else {
            if let Some(ids) = plan.select {
                self.call(SelectionCall::Layers { ids, add: false }.into(), cx);
            }
            return;
        };
        let select = plan.select;
        let Some(session) = &self.session else { return };
        let pending = self
            .kernel
            .call(&session.project, Request::new(Caller::native(), call));
        cx.spawn(async move |this, cx| {
            let result = pending.await;
            let _ = this.update(cx, |v, cx| match result {
                Some(Ok(ack)) => {
                    if let Reply::Throw(e) = ack.reply {
                        v.message = e.to_string();
                        return;
                    }
                    if let Some(ids) = select {
                        v.call(SelectionCall::Layers { ids, add: false }.into(), cx);
                    }
                }
                other => v.edit_failed(other.and_then(Result::err)),
            });
        })
        .detach();
    }
    pub fn time(&self) -> f64 {
        self.viewer
            .as_ref()
            .and_then(|v| v.status().clock)
            .map_or(0., |c| c.media_time.get() as f64 / TICKS_PER_SECOND as f64)
    }
    pub(crate) fn event_time() -> f64 {
        if let Some(t) = DISPATCHING.get() {
            return if t > 0. {
                t
            } else {
                pm_engine_service::trace::now()
            };
        }
        objc2::MainThreadMarker::new()
            .and_then(|main| objc2_app_kit::NSApplication::sharedApplication(main).currentEvent())
            .map(|e| e.timestamp())
            .filter(|t| t.is_finite() && *t > 0.)
            .unwrap_or_else(pm_engine_service::trace::now)
    }
    pub fn seek(&mut self, seconds: f64) {
        self.seek_at(seconds, Self::event_time());
    }
    pub fn seek_at(&mut self, seconds: f64, input: f64) {
        let mut target = None;
        if let Some(v) = &self.viewer {
            let frame = self
                .rate
                .frame_of((seconds.clamp(0., self.duration) * TICKS_PER_SECOND as f64) as i64);
            if let Ok(time) = self.rate.frame_start(frame) {
                target = Some(time as f64 / TICKS_PER_SECOND as f64);
                if let Err(e) = v.scrub_at(Ticks::new(time).unwrap_or(Ticks::ZERO), input) {
                    self.message = e.to_string();
                }
            }
        }
        // The viewer's clock lands later (always so for a viewer bound a
        // moment ago, as on open): record the time asked for, so MCP's
        // playhead and the saved transport never read the old one.
        let _ = self.write_ui_at(target.unwrap_or_else(|| self.time()));
    }
    pub fn persist_ui(&self) {
        let _ = self.write_ui();
    }
    /// Resolves once the kernel has taken the write.
    pub(crate) fn write_ui(
        &self,
    ) -> Option<pm_kernel::Pending<Result<pm_kernel::state::StateAck, pm_kernel::KernelFailure>>>
    {
        self.write_ui_at(self.time())
    }
    fn write_ui_at(
        &self,
        time: f64,
    ) -> Option<pm_kernel::Pending<Result<pm_kernel::state::StateAck, pm_kernel::KernelFailure>>>
    {
        let (Some(s), Some(p)) = (&self.session, &self.published) else {
            return None;
        };
        let mut ui = p.ui_state.clone();
        // A project saved without viewer or transport state still records them.
        for value in [&mut ui.transport, &mut ui.viewer] {
            if !matches!(value, pm_model::Value::Object(_)) {
                *value = pm_model::Value::Object(Default::default());
            }
        }
        if let pm_model::Value::Object(m) = &mut ui.transport {
            m.insert("time", pm_model::Value::from(time));
            m.insert(
                "playing",
                pm_model::Value::from(
                    self.viewer
                        .as_ref()
                        .and_then(|v| v.status().clock)
                        .is_some_and(|c| c.state != TransportState::Stopped),
                ),
            );
        }
        if let pm_model::Value::Object(m) = &mut ui.viewer {
            m.insert("zoom", pm_model::Value::from(self.view.zoom.unwrap_or(1.)));
            m.insert("fit", pm_model::Value::from(self.view.zoom.is_none()));
            m.insert(
                "pan",
                pm_model::Value::Array(vec![self.view.pan[0].into(), self.view.pan[1].into()]),
            );
        }
        Some(self.kernel.state_transaction(StateTransaction {
            ops: vec![StateOp::Ui {
                project: s.project.clone(),
                next: ui,
            }],
            ..Default::default()
        }))
    }
    pub fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.open(SessionSource::File(path), cx);
    }
    fn open(&mut self, source: SessionSource, cx: &mut Context<Self>) {
        self.open_named(source, None, cx);
    }
    fn open_named(&mut self, source: SessionSource, name: Option<String>, cx: &mut Context<Self>) {
        if self.opening || self.recovery_pending || self.close_pending || self.save_pending {
            self.message = "Finish the active edit or recovery decision first".into();
            return;
        }
        // A window's project is never replaced by an open: the file joins
        // the window's tabs and the project shown stays as it is.
        if self.session.is_some() {
            match source {
                SessionSource::File(path) => {
                    let leader = self.shell_window_id(cx);
                    crate::entry::open_in_new_tab(crate::entry::Pending::File(path), leader, cx);
                }
                _ => self.message = "Close this project before opening another here.".into(),
            }
            cx.notify();
            return;
        }
        let Some(operation) = self.mcp.try_operation() else {
            self.message =
                "A project operation is in progress. Try opening again when it finishes.".into();
            cx.notify();
            return;
        };
        let open_id = match &source {
            SessionSource::Library { project, .. } => project.clone(),
            SessionSource::File(p) => stable_id(p),
            SessionSource::New(_) => pm_kernel::new_id("app"),
        };
        let name = name.unwrap_or_else(|| match &source {
            SessionSource::File(path) => path
                .file_stem()
                .map_or("Untitled".into(), |n| n.to_string_lossy().into_owned()),
            SessionSource::Library { project, .. } => self
                .recent
                .iter()
                .find(|(id, _)| id == project)
                .map_or("Untitled".into(), |(_, name)| name.clone()),
            SessionSource::New(_) => "Untitled".into(),
        });
        let path = match &source {
            SessionSource::File(path) => Some(path.clone()),
            _ => None,
        };
        let electron_profile = path.is_some().then(|| self.electron.clone()).flatten();
        let pending = self.kernel.shell(ShellCall::PrepareOpen {
            id: open_id,
            source,
            electron_profile,
        });
        let saved = path.as_ref().is_none_or(|p| p.exists());
        self.settle_open(operation, name, path, saved, pending, cx);
    }
    /// A project the Library's worker already opened.
    pub fn adopt_reply(
        &mut self,
        reply: ShellReply,
        name: String,
        file: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        // This window shows a project (or is opening one): the opened one
        // joins its tabs, and Home gives way to the project behind it.
        if self.session.is_some() || self.opening {
            let leader = self.shell_window_id(cx);
            let reply = Box::new(reply);
            crate::entry::open_in_new_tab(
                crate::entry::Pending::Adopt { reply, name, file },
                leader,
                cx,
            );
            self.hide_home(cx);
            return;
        }
        let Some(operation) = self.mcp.try_operation() else {
            self.message =
                "A project operation is in progress. Try opening again when it finishes.".into();
            cx.notify();
            // Nothing will adopt this session, so it can't stay open in the kernel.
            if let ShellReply::Opened { session, .. } = reply {
                let closing = self.kernel.shell(ShellCall::Close(session));
                cx.spawn(async move |_, _| drop(closing.await)).detach();
            }
            return;
        };
        // A library project's saved version is in the library, wherever its file went.
        self.settle_open(
            operation,
            name,
            file,
            true,
            async move { Some(Ok(reply)) },
            cx,
        );
    }
    fn settle_open(
        &mut self,
        operation: crate::mcp::Operation,
        name: String,
        path: Option<PathBuf>,
        saved: bool,
        pending: impl std::future::Future<Output = Option<Result<ShellReply, KernelFailure>>> + 'static,
        cx: &mut Context<Self>,
    ) {
        // Only a window without a project opens one (`open_named`,
        // `adopt_reply`): nothing is closed or saved to make room.
        debug_assert!(self.session.is_none());
        let kernel = self.kernel.clone();
        let after_stop = std::mem::take(&mut self.reopening_stopped);
        // Commands still waiting on an earlier open never reach this project.
        let (open, dropped) = self.after_open.begin(&name);
        self.opening = true;
        self.busy = Some(crate::platform::Busy::begin("Opening a project"));
        self.message = dropped.map_or_else(
            || format!("Opening “{name}”…"),
            |note| format!("{note} Opening “{name}”…"),
        );
        self.opening_name = Some(name.clone());
        self.open_timer.begin();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let _operation = operation;
            let mut result = pending.await;
            if let Some(Ok(reply @ ShellReply::RecoveryRequired { .. })) = &result {
                let recovery = crate::surfaces::recovery::PreparedRecovery::from_reply(
                    reply, &name, false, saved,
                )
                .map(|mut r| {
                    let asks = r
                        .alert
                        .buttons
                        .iter()
                        .any(|b| b.action == crate::surfaces::recovery::Action::Recover);
                    if after_stop && asks {
                        r.alert = crate::surfaces::recovery::after_stop(r.alert, &name);
                    }
                    r
                });
                if let Some(recovery) = recovery {
                    let answer = this.update(cx, |v, _| {
                        if matches!(
                            reply,
                            ShellReply::RecoveryRequired {
                                recovery: RecoveryResult::Unrecoverable { .. },
                                ..
                            }
                        ) {
                            v.errors
                                .borrow_mut()
                                .push("Project", &recovery.alert.message);
                        }
                        v.recovery_pending = true;
                        v.mcp.awaiting_recovery(Some(name.clone()));
                        recovery.alert.sheet(&v.host.borrow().window)
                    });
                    let response = match answer {
                        Ok(rx) => rx.recv().await.ok().flatten(),
                        Err(_) => None,
                    };
                    let _ = this.update(cx, |v, _| v.mcp.awaiting_recovery(None));
                    if let Some(call) = response.and_then(|action| recovery.decision(action)) {
                        result = kernel.shell(call).await;
                    }
                }
            }
            let _ = this.update(cx, |v, cx| {
                v.opening = false;
                v.recovery_pending = false;
                // Panels say it's opening until they've mounted for the project.
                v.settling = match &result {
                    Some(Ok(ShellReply::Opened { session, .. })) => Some(session.project.clone()),
                    _ => None,
                };
                v.settling_open = open;
                // Held until the panels have drawn the project (`settled`).
                if v.settling.is_none() {
                    v.opening_name = None;
                    v.busy = None;
                    // No project arrived; its open failure says why.
                    if !matches!(&result, Some(Ok(ShellReply::Opened { .. }))) {
                        v.after_open.fail();
                    }
                }
                match result {
                    Some(Ok(ShellReply::Opened {
                        session,
                        recovery: _,
                        decision,
                        info,
                    })) => {
                        v.open_timer.mark("opened", serde_json::json!({}));
                        // Media resolves on a worker after the project shows.
                        let in_place = decision == pm_kernel::locks::OpenDecision::Writable;
                        v.adopt(session, decision, info.history_rejected, cx);
                        v.report_first_frame(cx);
                        v.load_recent(cx);
                        v.offer_stranded_copies(cx);
                        // No lock reaches another computer through a sync
                        // client: say what does guard the file.
                        if in_place && v.path.as_deref().is_some_and(pm_kernel::locks::is_synced) {
                            v.message = SYNCED_FOLDER.into();
                        }
                    }
                    // Already open in a window: that window comes forward.
                    Some(Err(KernelFailure::AlreadyOpen(project))) => {
                        v.message = String::new();
                        v.focus_project(&project, cx);
                    }
                    Some(Err(error)) => {
                        v.open_timer
                            .mark("failed", serde_json::json!({"message": error.to_string()}));
                        v.report_open_failure(&name, path, &error, cx)
                    }
                    Some(Ok(ShellReply::RecoveryRequired { .. })) => {
                        v.message = format!("“{name}” wasn’t opened. Its recovery data is kept.");
                    }
                    other => {
                        v.message = format!("“{name}” couldn’t be opened.");
                        v.errors.borrow_mut().push(
                            "Project",
                            &format!(
                                "“{name}” couldn’t be opened: {}",
                                crate::lifecycle::reason(other)
                            ),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// A file that opened in place: if the library holds copies with work
    /// for it that the person hasn't been told about, say so once and offer
    /// the newest (it opens as a tab; the file stays open here). A file that
    /// opened as a new copy because an earlier copy under its name was made
    /// of another file is told about that earlier copy the same way.
    fn offer_stranded_copies(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let asked = self.kernel.shell(ShellCall::CopiesOf(session));
        cx.spawn(async move |this, cx| {
            let Some(Ok(ShellReply::Copies(rows))) = asked.await else {
                return;
            };
            let field = |row: &pm_model::Value, key: &str| {
                row.get(key).and_then(pm_model::Value::as_str).map(str::to_owned)
            };
            let names: Vec<String> = rows.iter().filter_map(|r| field(r, "name")).collect();
            let Some(newest) = rows.first().and_then(|r| field(r, "id")) else {
                return;
            };
            let different = rows.iter().any(|r| r.get("different").is_some());
            let named = rows.first().and_then(|r| field(r, "file"));
            let Ok(answer) = this.update(cx, |v, _| {
                let file = named
                    .or_else(|| {
                        v.path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                    })
                    .unwrap_or_default();
                if different {
                    v.errors.borrow_mut().push(
                        "Project",
                        &format!(
                            "“{file}” isn’t the file an earlier copy was made from. That copy is still in your projects: “{}”.",
                            names.join("”, “")
                        ),
                    );
                    return earlier_copy_alert(&file, &names).sheet(&v.host.borrow().window);
                }
                v.errors.borrow_mut().push(
                    "Project",
                    &format!(
                        "Your projects hold a copy of “{file}” with work that isn’t in the file: “{}”.",
                        names.join("”, “")
                    ),
                );
                stranded_copies_alert(&file, &names).sheet(&v.host.borrow().window)
            }) else {
                return;
            };
            if answer.recv().await.ok().flatten() == Some(true) {
                let _ = this.update(cx, |v, cx| {
                    let leader = v.shell_window_id(cx);
                    crate::entry::open_in_new_tab(crate::entry::Pending::Library(newest), leader, cx);
                });
            }
        })
        .detach();
    }
    /// `PM_NATIVE_MAIN_TRACE`: when the viewer first presents the opened project.
    fn report_first_frame(&mut self, cx: &mut Context<Self>) {
        if !self.open_timer.active() {
            return;
        }
        let Some(doc) = self.published.as_ref().map(|p| p.snapshot.doc_id) else {
            return;
        };
        let timer = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            for _ in 0..24_000 {
                let shown = this.update(cx, |v, _| {
                    let status = v.viewer.as_ref().map(|v| v.status());
                    let frame = status.as_ref().and_then(|s| s.frame.as_ref());
                    if let Some(f) = frame.filter(|f| f.stamp.doc_id == doc && f.gpu_ok) {
                        v.open_timer
                            .mark("first_frame", serde_json::json!({"cache_hit": f.cache_hit}));
                        return true;
                    }
                    if let Some(error) = status.and_then(|s| s.error) {
                        v.open_timer
                            .mark("viewer_error", serde_json::json!({"message": error}));
                        return true;
                    }
                    false
                });
                if shown.unwrap_or(true) {
                    break;
                }
                timer.timer(std::time::Duration::from_millis(5)).await;
            }
        })
        .detach();
    }
    /// Brings forward the window that shows `project` (this one, or another).
    /// Opening a project that's already open does this, as on every Mac app.
    pub fn focus_project(&mut self, project: &str, cx: &mut Context<Self>) {
        let Some(key) = self.mcp.window_of(project) else {
            return;
        };
        if key == self.window_key {
            gpui_macos::child_window::raise_if_active(&self.host.borrow().window);
            return;
        }
        // Only while the app is active, as for this window: activating the
        // window activates the app, which would bring a background app (a
        // launch restoring its windows that is also handed one of their
        // files, an agent's open) in front of the one the person is using.
        if let Some(editor) = crate::entry::editor(key, cx) {
            gpui_macos::child_window::raise_if_active(&editor.read(cx).host.borrow().window);
        }
    }
    /// An internal error stopped this window's project: the kernel takes no
    /// more edits or saves for it. Reopen closes it (nothing is written) and
    /// opens it again, where the recovery prompt brings back every change
    /// made before the error.
    fn offer_reopen(&mut self, p: &pm_kernel::Published, cx: &mut Context<Self>) {
        use crate::surfaces::documents::{Action, stopped};
        let name = p
            .snapshot
            .root()
            .and_then(|r| r.get("name"))
            .and_then(crate::node_string)
            .unwrap_or("Untitled")
            .to_owned();
        self.message = format!("“{name}” stopped after an internal error. Reopen it to recover.");
        let rx = stopped(&name).sheet(&self.host.borrow().window);
        cx.spawn(async move |this, cx| {
            if rx.recv().await.ok().flatten() == Some(Action::Reopen) {
                let _ = this.update(cx, |v, cx| v.reopen_stopped(name, cx));
            }
        })
        .detach();
    }
    /// Closes the stopped project (the kernel writes nothing and keeps its
    /// journal) and opens it again through the recovery prompt.
    pub(crate) fn reopen_stopped(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let Some(p) = self
            .kernel
            .published(&session.project)
            .filter(|p| p.fault.is_some())
        else {
            return;
        };
        let Some(source) = reopen_source(&p, &self.profile) else {
            return;
        };
        if self.opening || self.recovery_pending || self.close_pending || self.save_pending {
            self.message = "Finish the active open or recovery decision first".into();
            cx.notify();
            return;
        }
        let closing = self.kernel.shell(ShellCall::Close(session));
        cx.spawn(async move |this, cx| {
            let closed = matches!(closing.await, Some(Ok(ShellReply::Closed { .. })));
            let _ = this.update(cx, |v, cx| {
                if !closed {
                    v.message = format!(
                        "“{name}” couldn’t be closed to reopen it; its recovery data is kept."
                    );
                    cx.notify();
                    return;
                }
                if let Some(viewer) = v.viewer.take() {
                    viewer.retire();
                }
                v.session = None;
                v.published = None;
                v.mirror = None;
                v.mcp.bind_window(v.window_key, None);
                v.reopening_stopped = true;
                v.open_named(source, Some(name), cx);
            });
        })
        .detach();
    }
    /// S004: an alert naming the file, with Show in Finder when it still exists.
    fn report_open_failure(
        &mut self,
        name: &str,
        path: Option<PathBuf>,
        error: &KernelFailure,
        cx: &mut Context<Self>,
    ) {
        use crate::surfaces::documents::{self, Action};
        let exists = path.as_ref().is_some_and(|p| p.exists());
        let alert = documents::open_error(name, open_failure(error), exists).notice();
        self.errors.borrow_mut().push("Project", &alert.message);
        self.message = String::new();
        let rx = alert.sheet(&self.host.borrow().window);
        cx.spawn(async move |_, _| {
            if rx.recv().await.ok().flatten() == Some(Action::ShowInFinder)
                && let Some(path) = path
            {
                platform::reveal(&path);
            }
        })
        .detach();
    }
    fn adopt(
        &mut self,
        session: SessionHandle,
        decision: pm_kernel::locks::OpenDecision,
        history_rejected: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(viewer) = self.viewer.take() {
            viewer.retire();
        }
        self.host.borrow_mut().close_flowchart();
        self.composition_views.clear();
        self.rows.clear();
        self.keyframes = 0;
        self.session = Some(session);
        self.media_refresh.reset();
        self.media = MediaCatalog::default();
        self.media_base = MediaCatalog::default();
        self.media_resolved = false;
        self.mirror = None;
        self.selected = None;
        self.refresh(cx);
        // The status bar shows document state and counts; only exceptions get words.
        let notice = self.errors.borrow_mut().take_notice();
        self.message = [
            copy_notice(&decision).map(str::to_owned),
            history_rejected
                .then(|| "Undo history from the last session couldn’t be restored.".to_owned()),
            notice,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
        self.bind_composition();
        self.apply_viewer_options();
        self.rebuild_popout_viewer();
        if let Some(ui) = self.published.as_ref().map(|p| p.ui_state.clone()) {
            let time = ui
                .transport
                .get("time")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.);
            self.seek(time);
        }
        self.mcp.bind_window(self.window_key, self.session.clone());
    }
    fn bind_composition(&mut self) {
        // The viewer before this one changes places with the next in one
        // step (`replace_viewer`): retired first, its document would be
        // without a view for a moment, and the engine closes a document
        // without views (its override answers, caches and programs go).
        let old = self.viewer.take();
        let retire = |old: Option<Viewer>| {
            if let Some(viewer) = old {
                viewer.retire();
            }
        };
        let Some(p) = self.published.clone() else {
            return retire(old);
        };
        let comp = p.snapshot.render_comp();
        // A document without a root has nothing to view.
        let Some(root) = p.snapshot.document.root() else {
            return retire(old);
        };
        let composition = pm_kernel::model::comp_node(root, Some(&comp)).unwrap_or(root);
        let number = |key: &str, default| {
            composition
                .get(key)
                .and_then(crate::node_number)
                .unwrap_or(default)
        };
        self.duration = number("dur", 1.).max(0.001);
        self.dimensions = composition_size(number("w", 1920.), number("h", 1080.));
        self.rate = composition_rate(composition);
        let media = self.media.clone();
        self.background = composition
            .get("bg")
            .and_then(crate::node_string)
            .and_then(panels::parse_background);
        let config = ViewerConfig {
            snapshot: p.snapshot.clone(),
            composition: comp,
            media,
            output: pm_engine_service::Output {
                width: 640,
                height: 360,
                display: 0,
                headroom: 1.,
                crop: None,
            },
            layer: Some(self.host.borrow().viewer_layer()),
            rate: self.rate,
            end: Some((self.duration * TICKS_PER_SECOND as f64) as i64),
            audio: true,
        };
        let placed = match &old {
            Some(old) => self.engine.replace_viewer(old, config),
            None => self.engine.viewer(config),
        };
        match placed {
            Ok(viewer) => {
                // The editor's viewer drags a slow project at reduced resolution.
                let _ = viewer.adapt_drags(true);
                let _ = viewer.skim_audio(self.skimming.on && self.skimming.audio);
                self.skimming.at = None;
                self.viewer = Some(viewer)
            }
            Err(e) => {
                retire(old);
                self.message = format!("Viewer failed: {e}")
            }
        }
        self.apply_extension_resources();
    }
    /// MCP placed a project in this window (open_project, create_project): an
    /// empty window shows it; a window that shows a project keeps it and the
    /// placed one joins its tabs, as opening from the window does. Other
    /// windows' projects are theirs; this never follows them.
    pub fn follow_mcp(&mut self, cx: &mut Context<Self>) {
        if self.opening || self.close_pending {
            return;
        }
        // Taken atomically: an MCP open racing this one can't close it now.
        let Some(session) = self.mcp.claim_placed(self.window_key) else {
            return;
        };
        if Some(&session) == self.session.as_ref() {
            return;
        }
        if self.kernel.published(&session.project).is_none() {
            // Gone before the window took it: keep showing what it showed.
            self.mcp.bind_window(self.window_key, self.session.clone());
            return;
        }
        if self.session.is_some() {
            // This window still shows its own project.
            self.mcp.bind_window(self.window_key, self.session.clone());
            let leader = self.shell_window_id(cx);
            crate::entry::open_in_new_tab(crate::entry::Pending::Session(session), leader, cx);
            return;
        }
        self.adopt(session, pm_kernel::locks::OpenDecision::Writable, false, cx);
        cx.notify();
    }
    pub fn load_recent(&mut self, cx: &mut Context<Self>) {
        let pending = self.kernel.shell(ShellCall::Recent);
        cx.spawn(async move |this, cx| {
            if let Some(Ok(ShellReply::Recent(rows))) = pending.await {
                let _ = this.update(cx, |v, cx| {
                    v.recent = rows
                        .iter()
                        .filter_map(|r| {
                            Some((
                                r.get("id")?.as_str()?.into(),
                                r.get("name")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Untitled")
                                    .into(),
                            ))
                        })
                        .collect();
                    cx.notify();
                });
            }
        })
        .detach();
    }
    pub fn save(&mut self, as_path: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.recovery_pending || self.save_pending {
            self.message = "Finish the recovery decision or current save first".into();
            return;
        }
        let path = as_path.clone();
        if self.finish_edits_then(cx, move |v, cx| v.save(path, cx)) {
            return;
        }
        let Some(session) = self.session.clone() else {
            return;
        };
        self.save_pending = true;
        self.saving_note = true;
        self.message = SAVING.into();
        cx.notify();
        self.persist_ui();
        let kernel = self.kernel.clone();
        let file_save = as_path.is_some()
            || self.published.as_ref().is_some_and(|p| {
                p.session
                    .as_ref()
                    .is_some_and(|s| matches!(s.destination, SessionDestination::File(_)))
            });
        cx.spawn(async move |this, cx| {
            let result = if file_save {
                // The save panel already confirmed replacing an existing file.
                let prepared = match as_path.clone() {
                    Some(path) => kernel.prepare_save_replacing(session, path),
                    None => kernel.prepare_save(session, None),
                };
                match prepared.await {
                    Some(Ok(save)) => {
                        let completion = cx
                            .background_executor()
                            .spawn(async move { save.run() })
                            .await;
                        kernel.finish_save(completion).await
                    }
                    Some(Err(e)) => Some(Err(e)),
                    None => None,
                }
            } else {
                kernel.shell(ShellCall::Save(session)).await
            };
            let _ = this.update(cx, |v, cx| {
                v.save_pending = false;
                let close = v.close_after_save.take();
                // A save that kept another version beside the file has
                // something to say first: the window stays.
                let saved = matches!(
                    &result,
                    Some(Ok(ShellReply::Saved {
                        recovery_copy: None,
                        displaced: None,
                        ..
                    }))
                );
                match result {
                    Some(Ok(ShellReply::Saved {
                        session,
                        recovery_copy,
                        displaced,
                    })) => {
                        v.session = Some(session);
                        if as_path.is_some() {
                            v.path = as_path;
                        }
                        v.unprotected = false;
                        v.message = recovery_copy.map_or(String::new(), |_| {
                            "Its file couldn’t be written, so your changes were saved as a recovered copy in your projects.".into()
                        });
                        v.load_recent(cx);
                        if let Some(kept) = displaced {
                            v.other_version_kept(kept, cx);
                        }
                        v.write_poster(cx);
                    }
                    other => v.save_failed(&crate::lifecycle::reason(other), cx),
                }
                v.refresh(cx);
                if let Some(intent) = close {
                    if saved {
                        v.complete_close(intent, cx);
                    } else {
                        v.cancel_close(cx);
                    }
                }
            });
        })
        .detach();
    }
    fn project_name(&self) -> String {
        self.published
            .as_ref()
            .and_then(|p| p.snapshot.document.root())
            .and_then(|r| r.get("name"))
            .and_then(crate::node_string)
            .unwrap_or("Untitled")
            .to_owned()
    }
    /// The save went through, and a version that arrived meanwhile was kept
    /// beside the file: a sheet that names it, a way to it, an Error Log row.
    fn other_version_kept(&mut self, kept: PathBuf, cx: &mut Context<Self>) {
        use crate::surfaces::documents::{Action, other_version_kept};
        let name = self.project_name();
        let file_name = |path: &std::path::Path| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let file = self.path.as_deref().map(file_name).unwrap_or_default();
        self.message = format!("“{name}” was saved. Another version of its file was kept.");
        self.errors.borrow_mut().push(
            "Save",
            &format!(
                "While “{name}” was being saved, another version of its file arrived. It was kept at {}",
                kept.display()
            ),
        );
        let answer =
            other_version_kept(&name, &file, &file_name(&kept)).sheet(&self.host.borrow().window);
        cx.spawn(async move |_, _| {
            if answer.recv().await.ok().flatten() == Some(Action::ShowInFinder) {
                platform::reveal(&kept);
            }
        })
        .detach();
    }
    /// A save that did not happen is a sheet in plain words (S5): Save As
    /// and Show in Finder are one press away, and the kernel's own account
    /// is an Error Log row, never copy.
    fn save_failed(&mut self, detail: &str, cx: &mut Context<Self>) {
        use crate::surfaces::documents::{Action, WriteFailure, save_failed};
        let name = self.project_name();
        self.message = format!("“{name}” couldn’t be saved.");
        self.errors
            .borrow_mut()
            .push("Save", &format!("“{name}” couldn’t be saved: {detail}"));
        let path = self.path.clone().filter(|p| p.exists());
        let answer = save_failed(&name, WriteFailure::of(detail), path.is_some())
            .sheet(&self.host.borrow().window);
        cx.spawn(
            async move |this, cx| match answer.recv().await.ok().flatten() {
                Some(Action::SaveAs) => {
                    let _ = this.update(cx, |v, cx| v.save_dialog(cx));
                }
                Some(Action::ShowInFinder) => {
                    if let Some(path) = path {
                        platform::reveal(&path);
                    }
                }
                _ => {}
            },
        )
        .detach();
    }
    /// An edit the kernel refused or could not answer: plain words in the
    /// status bar, the kernel's account in the Error Log.
    fn edit_failed(&mut self, error: Option<KernelFailure>) {
        self.message = "That change couldn’t be made.".into();
        let detail =
            error.map_or_else(|| "the project didn’t answer".to_owned(), |e| e.to_string());
        self.errors
            .borrow_mut()
            .push("Project", &format!("A change couldn’t be made: {detail}"));
    }
    /// What the kernel says about this window's project, in the person's
    /// words. A change that could not be written to the recovery data is the
    /// one moment unsaved work isn't protected: a sheet, an Error Log row,
    /// and the subtitle until a save succeeds.
    fn notice(&mut self, notice: pm_kernel::Notice, cx: &mut Context<Self>) {
        use pm_kernel::Notice::*;
        match notice {
            NotDurable { message } => {
                use crate::surfaces::documents::{Action, WriteFailure, not_protected};
                let name = self.project_name();
                self.errors.borrow_mut().push(
                    "Recovery",
                    &format!(
                        "A change to “{name}” couldn’t be written to its recovery data: {message}"
                    ),
                );
                self.message =
                    format!("Powermove couldn’t protect your latest change to “{name}”. Save now.");
                if std::mem::replace(&mut self.unprotected, true) {
                    // Already said, and still showing in the subtitle.
                    return;
                }
                let answer = not_protected(&name, WriteFailure::of(&message))
                    .sheet(&self.host.borrow().window);
                cx.spawn(async move |this, cx| {
                    if answer.recv().await.ok().flatten() == Some(Action::Save) {
                        let _ = this.update(cx, |v, cx| v.save(None, cx));
                    }
                })
                .detach();
            }
            CommitWithNotice { label, reason } => {
                self.message =
                    format!("“{label}” was kept: cancelling it would have removed later work.");
                self.errors
                    .borrow_mut()
                    .push("Project", &format!("“{label}” was kept: {reason}"));
            }
            ForeignSplit { label, writes } => {
                self.message = format!(
                    "“{label}” was cancelled. {writes} change{} made meanwhile by something else {} kept as a separate undo step.",
                    if writes == 1 { "" } else { "s" },
                    if writes == 1 { "was" } else { "were" }
                );
            }
            Sealed { label, .. } => {
                self.message = format!("“{label}” was finished for an extension that stopped.");
            }
            Barrier { label } => {
                self.message = format!(
                    "Undo stopped at “{label}”: the extension that made it is no longer running."
                );
            }
        }
    }
    pub fn save_dialog(&mut self, cx: &mut Context<Self>) {
        let rx = platform::paths(&self.host.borrow().window, true);
        cx.spawn(async move |this, cx| {
            if let Ok(Some(path)) = rx.recv().await {
                let _ = this.update(cx, |v, cx| v.save(Some(path), cx));
            }
        })
        .detach();
    }
    pub fn command(
        &mut self,
        cmd: &ShellCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if crate::generated_panel::pick(self, &cmd.name, cx) {
            return true;
        }
        if let Some(id) = cmd.name.strip_prefix("takeRestore:") {
            self.restore_take(id, cx);
            return true;
        }
        if let Some(id) = cmd.name.strip_prefix("takeDelete:") {
            self.delete_take(id, cx);
            return true;
        }
        if cmd.name == "toolShape" {
            // The segment menu names a variant; Q again on the Shape tool
            // cycles them; a toolbar click keeps the current one.
            let variants = pm_shell::SHAPES.map(|(v, _, _)| v);
            let at = variants
                .iter()
                .position(|v| *v == self.shape_variant)
                .unwrap_or(0);
            if let Some(v) = cmd.args.first().and_then(|v| v.as_str())
                && variants.contains(&v)
            {
                self.shape_variant = v.into();
            } else if cmd.token.is_some() && self.tool == "toolShape" {
                self.shape_variant = variants[(at + 1) % variants.len()].into();
            }
            self.host.borrow().select_shape(&self.shape_variant);
        }
        if cmd.name.starts_with("tool") && cmd.name.len() > 4 {
            self.tool = cmd.name.clone();
            // The Pen tool shows the selection's paths; others hide them.
            if let Some(gizmos) = self.builtins.gizmos.clone() {
                gizmos.update(cx, |g, cx| g.refresh(cx));
            }
            // The `tool` service names it as v1 did: toolSelect is "select".
            if let Some(host) = &self.extensions.host {
                let name = &cmd.name[4..];
                let tool = name[..1].to_lowercase() + &name[1..];
                host.shell_state(serde_json::json!({ "tool": tool }));
            }
            return false;
        }
        // The menu or a launch argument during a long open: the project is
        // coming, so the command waits for it rather than finding none.
        if (self.opening || self.settling.is_some())
            && crate::layer_commands::waits_for_project(&cmd.name)
        {
            self.after_open.defer(cmd.name.clone(), cmd.args.clone());
            return true;
        }
        if self.extension_command(cmd, cx) {
            return true;
        }
        // Edit menu items reach a focused text field before the document.
        if cmd.name.starts_with("context") && self.host.borrow().system(&cmd.name) {
            return true;
        }
        // Escape on Home clears its search, then returns to the project (v1).
        if matches!(cmd.name.as_str(), "deselect" | "cancel") && self.home_visible {
            match self.library.clone() {
                Some(library) if !library.read(cx).controller.query.text.is_empty() => {
                    library.update(cx, |v, cx| v.clear_search(cx));
                }
                _ => self.hide_home(cx),
            }
            return true;
        }
        if matches!(cmd.name.as_str(), "deselect" | "cancel") && self.cancel_gesture(cx) {
            return true;
        }
        use crate::layer_commands::Route;
        match crate::layer_commands::route(&cmd.name, &cmd.args) {
            Some(Route::Plan(name)) => return self.run_plan(name, &cmd.args, cx),
            Some(Route::Copy { cut }) => return self.copy_selection(cut, cx),
            Some(Route::Paste) => return self.paste_clipboard(cx),
            Some(Route::Timeline { frames, shift }) => {
                let Some(timeline) = self.builtins.timeline.clone() else {
                    return false;
                };
                let time = self.time();
                timeline.update(cx, |t, cx| t.command(&cmd.name, time, frames, shift, cx));
                return true;
            }
            None => {}
        }
        match cmd.name.as_str() {
            "v4.platform" => self.builtin_platform(window, cx),
            "text.editSelected" => {
                let Some(gizmos) = self.builtins.gizmos.clone() else {
                    return false;
                };
                return gizmos.update(cx, |g, cx| g.edit_selected_text(window, cx));
            }
            "toggleSkimming" => self.toggle_skimming(cx),
            "toggleAudioSkimming" => self.toggle_audio_skimming(cx),
            "toggleLayerControls" => {
                if let Some(gizmos) = self.builtins.gizmos.clone() {
                    let shown = gizmos.update(cx, |g, cx| {
                        g.toggle_controls(cx);
                        g.controls()
                    });
                    if let Some(host) = &self.extensions.host {
                        host.shell_state(serde_json::json!({ "showControls": shown }));
                    }
                }
            }
            "settings" | "agent" => self.show_builtin(&cmd.name, window, cx),
            "quit" => cx.defer(crate::lifecycle::begin_quit),
            "closeWindow" | "closeTab" => self.request_close(crate::lifecycle::Intent::Window, cx),
            "cancel" | "deselect" if self.importing.is_some() => {
                if let Some(cancel) = &self.importing {
                    cancel.store(true, std::sync::atomic::Ordering::Release);
                }
            }
            "import" => self.pick_media(None, true, cx),
            "importFolder" => self.pick_media_folder(None, cx),
            "importSequence" => self.pick_sequence(cx),
            "addCaption" | "importCaptions" | "exportCaptions" | "generateCaptions"
            | "deleteCaptions" | "splitCaption" => {
                return self.captions_command(&cmd.name, window, cx);
            }
            "speechDelete" => {
                if let Some(id) = cmd.args.first().and_then(|v| v.as_str()) {
                    self.confirm_speech_delete(id, cx);
                }
            }
            "shader.compile" => return self.shader_command(&cmd.name, cx),
            name if name.starts_with("shader.preset:") => return self.shader_command(name, cx),
            name if name.starts_with("captions.cue:") => {
                return self.captions_command(name, window, cx);
            }
            name if name.starts_with("captions.layer:") => {
                return self.captions_command(name, window, cx);
            }
            "open" => {
                let rx = platform::paths(&self.host.borrow().window, false);
                cx.spawn(async move |this, cx| {
                    if let Ok(Some(path)) = rx.recv().await {
                        let _ = this.update(cx, |v, cx| v.open_file(path, cx));
                    }
                })
                .detach();
            }
            "save" => self.save(None, cx),
            "saveAs" => self.save_dialog(cx),
            "export" => self.show_builtin("export", window, cx),
            "undo" | "contextUndo" => self.call(Call::History(HistoryCall::Undo), cx),
            "redo" | "contextRedo" => self.call(Call::History(HistoryCall::Redo), cx),
            "precompose" => {
                let Some(published) = self.published.clone() else {
                    return false;
                };
                let Some(draft) = crate::layer_commands::precompose_draft(&published) else {
                    return false;
                };
                let time = self.time();
                let sheet = crate::dialogs::appkit::precompose_sheet(
                    &self.host.borrow().window,
                    &draft,
                    &pm_shell::tokens(),
                    objc2::MainThreadMarker::new().expect("menu commands run on the main thread"),
                );
                cx.spawn(async move |this, cx| {
                    let answer = sheet.result.recv().await;
                    drop(sheet.target);
                    if let Ok(Some(draft)) = answer {
                        let _ = this.update(cx, |v, cx| {
                            let plan =
                                crate::layer_commands::precompose_plan(&published, &draft, time);
                            v.run_layer_plan(plan, cx);
                        });
                    }
                })
                .detach();
            }
            "composition.flowchart" => {
                let mut host = self.host.borrow_mut();
                if cmd.token.is_some()
                    && !host.composition_canvas_focused()
                    && !host.flowchart_open()
                {
                    return false;
                }
                let Some(published) = self.published.as_ref() else {
                    return false;
                };
                let Some(chart) = pm_kernel::navigation::Flowchart::read(&published.snapshot)
                else {
                    return false;
                };
                use pm_shell::FlowchartRow;
                let mut rows = vec![FlowchartRow::Heading("Used In".into())];
                let mut group = |items: Vec<pm_kernel::navigation::Composition>, empty: &str| {
                    if items.is_empty() {
                        rows.push(FlowchartRow::Empty(empty.into()));
                    }
                    for item in items {
                        rows.push(FlowchartRow::Composition {
                            id: item.id,
                            name: item.name,
                            current: false,
                        });
                    }
                };
                group(chart.parents, "Not nested anywhere");
                rows.push(FlowchartRow::Composition {
                    id: chart.current.id,
                    name: chart.current.name,
                    current: true,
                });
                rows.push(FlowchartRow::Heading("Contains".into()));
                if chart.children.is_empty() {
                    rows.push(FlowchartRow::Empty("No nested compositions".into()));
                }
                for child in chart.children {
                    rows.push(FlowchartRow::Composition {
                        id: child.id,
                        name: child.name,
                        current: false,
                    });
                }
                return host.toggle_flowchart(rows, published.snapshot.doc_id);
            }
            "composition.open" => {
                let Some(id) = cmd.args.first().and_then(|a| a.as_str()) else {
                    return false;
                };
                let Some(document) = cmd.args.get(1).and_then(|a| a.as_u64()) else {
                    return false;
                };
                if self
                    .published
                    .as_ref()
                    .is_none_or(|p| p.snapshot.doc_id != document)
                {
                    return false;
                }
                if self
                    .published
                    .as_ref()
                    .and_then(|p| p.snapshot.open_comp())
                    .as_deref()
                    == Some(id)
                {
                    return true;
                }
                if let Some(viewer) = &self.viewer {
                    let _ = viewer.command(TransportCommand::Pause);
                }
                self.call(
                    Call::OpenComposition {
                        comp_id: id.into(),
                        document,
                    },
                    cx,
                );
            }
            "toggleLayerStrips" => {
                let Some(runtime) = self.builtins.runtime.clone() else {
                    return false;
                };
                let Ok(ticket) = runtime.reveal_masks() else {
                    return false;
                };
                cx.spawn(async move |this, cx| {
                    let result = ticket.await;
                    let _ = this.update(cx, |v, cx| {
                        if !v
                            .builtins
                            .runtime
                            .as_ref()
                            .is_some_and(|current| Arc::ptr_eq(current, &runtime))
                        {
                            return;
                        }
                        if let Err(error) = result {
                            v.message = format!("Mask properties could not be shown: {error}");
                        }
                        v.drain_builtins(cx);
                    });
                })
                .detach();
            }
            "fit" | "fitComp" | "fitComposition" | "viewer.fit" => {
                self.view.place(None, [0., 0.]);
                self.write_view(None, cx);
            }
            "actualSize" => {
                self.view.place(Some(1.), self.view.pan);
                self.write_view(None, cx);
            }
            "zoomIn" => self.step_zoom(1.25, cx),
            "zoomOut" => self.step_zoom(0.8, cx),
            "play" | "prevFrame" | "nextFrame" | "stepFrames" | "gotoStart" | "gotoEnd"
            | "shuttleJ" | "shuttleK" | "shuttleL" | "shuttleReverse" | "prevVisibleEvent"
            | "nextVisibleEvent" => {
                // Final Cut: Space plays from the skimmer, only while the
                // pointer is over the timeline now; otherwise from the playhead.
                let over = self
                    .builtins
                    .timeline
                    .as_ref()
                    .is_some_and(|t| t.read(cx).pointer_over(window));
                if cmd.name == "play" && !over && self.skimming.at.is_some() {
                    self.skim(None, Self::event_time());
                    if let Some(t) = self.builtins.timeline.clone() {
                        // The surface calls back into this controller.
                        cx.defer(move |cx| t.update(cx, |t, cx| t.skimming_changed(cx)));
                    }
                }
                if cmd.name == "play"
                    && over
                    && let Some(at) = self.skimming.at
                    && self.viewer.as_ref().is_some_and(|v| {
                        v.status()
                            .clock
                            .is_some_and(|c| c.state == TransportState::Stopped)
                    })
                {
                    self.seek(at);
                }
                let Some(v) = &self.viewer else { return false };
                let command = match cmd.name.as_str() {
                    "play" => {
                        if v.status()
                            .clock
                            .is_some_and(|c| c.state != TransportState::Stopped)
                        {
                            TransportCommand::Pause
                        } else {
                            TransportCommand::Play {
                                rate: PlaybackRate::NORMAL,
                            }
                        }
                    }
                    "prevFrame" => TransportCommand::Step { frames: -1 },
                    "nextFrame" => TransportCommand::Step { frames: 1 },
                    "stepFrames" => TransportCommand::Step {
                        frames: cmd.args.first().and_then(|v| v.as_i64()).unwrap_or(1) as i32,
                    },
                    "gotoStart" => {
                        self.seek(0.);
                        return true;
                    }
                    "gotoEnd" => {
                        self.seek((self.duration - 1. / self.rate.to_f64()).max(0.));
                        return true;
                    }
                    "prevVisibleEvent" | "nextVisibleEvent" => {
                        let time = self.time();
                        let mut times: Vec<_> = self
                            .layers()
                            .iter()
                            .flat_map(|l| [l.in_time, l.out_time])
                            .collect();
                        times.sort_by(f64::total_cmp);
                        let t = if cmd.name == "prevVisibleEvent" {
                            times.into_iter().rev().find(|t| *t < time - 0.00001)
                        } else {
                            times.into_iter().find(|t| *t > time + 0.00001)
                        };
                        if let Some(t) = t {
                            self.seek(t);
                        }
                        return true;
                    }
                    name => {
                        use pm_engine_service::transport::ShuttleKey::*;
                        let key = if name == "shuttleK" {
                            K
                        } else if name == "shuttleL" {
                            L
                        } else {
                            J
                        };
                        return v.shuttle(key, name == "shuttleReverse").unwrap_or(false);
                    }
                };
                if let Err(e) = v.command(command) {
                    self.message = e.to_string();
                }
            }
            name => return self.surface_command(name, cx),
        }
        cx.notify();
        true
    }
}
/// Where an open project comes from, to open it again.
pub(crate) fn reopen_source(
    p: &pm_kernel::Published,
    profile: &std::path::Path,
) -> Option<SessionSource> {
    Some(match &p.session.as_ref()?.destination {
        SessionDestination::File(path) => SessionSource::File(path.clone()),
        SessionDestination::Library(slot) => SessionSource::Library {
            profile: profile.to_owned(),
            project: slot.clone(),
        },
    })
}
/// Why the open failed, in the words the open alert has for it.
fn open_failure(error: &KernelFailure) -> crate::surfaces::documents::OpenFailure {
    use crate::surfaces::documents::OpenFailure;
    use pm_services::ServiceErrorCode::*;
    OpenFailure::Service(match error {
        KernelFailure::NoProject(_) => NotFound,
        // Unsaved work can't be protected without its journal: say so.
        KernelFailure::Journal(_) => return OpenFailure::Journal,
        // An open never faults a project; a stopped one only refuses edits.
        KernelFailure::Faulted(_) => Unavailable,
        KernelFailure::AlreadyOpen(_) | KernelFailure::Storage(_) => {
            let text = error.to_string().to_lowercase();
            if text.contains("permission") {
                PermissionDenied
            } else if text.contains("not found") || text.contains("no such file") {
                NotFound
            } else if text.contains("no space") {
                DiskFull
            } else {
                InvalidData
            }
        }
    })
}
pub const SAVING: &str = "Saving…";
/// `fpsRational`, else the stored float snapped to its exact rate: a project
/// saved at 29.97 plays on the 30000/1001 grid the engine draws.
fn composition_rate(composition: &pm_model::Node) -> pm_time::Rate {
    let rational = composition.get("fpsRational").map(|n| n.to_value());
    pm_model::ingest::IngestRate::from_document(
        pm_model::ingest::fps_rational_of(rational.as_ref()),
        composition.get("fps").and_then(crate::node_number),
    )
    .map_or(pm_time::Rate::FPS_30, |r| r.rate)
}

/// Status note for a save in flight, including saves started by MCP or the
/// kernel. Returns a replacement message on a transition only, so other
/// notes (refusals, errors) set during the save are kept until it ends.
pub fn save_note(was: bool, now: bool, dirty: bool, message: &str) -> Option<&'static str> {
    // Close/quit refusals explain why the window stayed open; keep them.
    let refusal = message.starts_with("Saving in progress")
        || message.starts_with("A project operation is in progress");
    match (was, now) {
        (false, true) if !refusal => Some(SAVING),
        (true, false) if message == SAVING || refusal => Some(if dirty {
            "Save did not complete; recovery data retained"
        } else {
            "Saved"
        }),
        _ => None,
    }
}
pub(crate) fn stable_id(path: &std::path::Path) -> String {
    // Stable across crashes; no random per-open journal namespace.
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .hash(&mut h);
    format!("file-{:016x}", h.finish())
}
impl Drop for Controller {
    fn drop(&mut self) {
        crate::diag::EDITORS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        if let Some(v) = &self.viewer {
            v.retire();
        }
        if let Some(p) = self.popout.take() {
            p.shutdown();
        }
    }
}
/// Points the engine's disk render cache at Storage › Renders (and finds its
/// frames again after Clear…), off the main thread.
pub(crate) fn sync_render_storage(
    pro_media: &Arc<crate::pro_media::ProMedia>,
    engine: &Arc<EngineService>,
) {
    let (pro_media, disk) = (pro_media.clone(), engine.render_cache());
    std::thread::Builder::new()
        .name("pm-render-storage".into())
        .spawn(move || {
            let _ = disk.set_storage(pro_media.render_frames());
        })
        .ok();
}

#[cfg(test)]
mod save_note_tests {
    use super::*;
    // `gpui::*` above exports a `test` macro; name the built-in explicitly.
    #[::core::prelude::v1::test]
    fn a_copy_says_what_it_is_for_as_long_as_its_window_shows_it() {
        let meta = pm_model::json::parse(
            r#"{"copyOf":{"path":"/Volumes/Share/job/cut.pmv","reason":"ReadOnly"}}"#,
        )
        .unwrap();
        assert_eq!(
            copy_subtitle(Some(&meta), ""),
            "Copy of cut.pmv (the file is read-only)"
        );
        assert_eq!(
            copy_subtitle(Some(&meta), "Edited"),
            "Copy of cut.pmv (the file is read-only) — Edited"
        );
        let plain = pm_model::json::parse("{}").unwrap();
        assert_eq!(copy_subtitle(Some(&plain), "Edited"), "Edited");
        assert_eq!(copy_subtitle(None, ""), "");
    }
    #[::core::prelude::v1::test]
    fn a_file_with_stranded_copies_offers_the_copy_and_keeps_the_file() {
        let alert = stranded_copies_alert("cut.pmv", &["cut — Native".to_owned()]);
        assert_eq!(
            alert.labels(),
            ["Open the Copy", "Keep Working in This File"]
        );
        assert!(alert.buttons[0].default && alert.buttons[0].action);
        assert!(!alert.buttons[1].action);
        assert!(alert.message.contains("a copy of “cut.pmv”"));
        assert!(alert.informative.contains("“cut — Native”"));
        let two = stranded_copies_alert("cut.pmv", &["a".to_owned(), "b".to_owned()]);
        assert!(two.message.contains("copies of") && two.informative.contains("“a” and 1 more"));
    }
    #[::core::prelude::v1::test]
    fn a_different_file_under_a_copied_name_is_told_about_the_earlier_copy() {
        let alert = earlier_copy_alert("cut.pmv", &["cut — Native".to_owned()]);
        assert_eq!(alert.labels(), ["Keep This One", "Open the Earlier Copy"]);
        // The file just opened stays unless the person asks for the other.
        assert!(alert.buttons[0].default && !alert.buttons[0].action);
        assert!(alert.buttons[1].action);
        assert!(alert.message.contains("“cut.pmv” isn’t the file"));
        assert!(alert.informative.contains("“cut — Native”"));
        assert!(alert.informative.contains("stays in your projects"));
    }
    #[::core::prelude::v1::test]
    fn a_background_recovery_failure_concerns_the_window_of_its_project() {
        assert!(unprotects("file-1", Some("file-1")));
        assert!(!unprotects("file-1", Some("file-2")));
        assert!(!unprotects("file-1", None));
    }
    #[::core::prelude::v1::test]
    fn a_composition_is_laid_out_at_a_positive_size_whatever_is_stored() {
        assert_eq!(composition_size(1920., 1080.), [1920., 1080.]);
        assert_eq!(composition_size(0., 1080.), [1., 1080.]);
        assert_eq!(composition_size(-5., f64::NAN), [1., 1.]);
        assert_eq!(composition_size(f64::INFINITY, 0.5), [1., 1.]);
    }
    #[::core::prelude::v1::test]
    fn a_composition_left_before_its_first_frame_keeps_the_time_asked_for() {
        // A viewer that reported a clock is where it says (playing included).
        assert_eq!(left_at(Some(3.5), Some(12.)), 3.5);
        // One bound a moment ago has none yet: the seek that restored it.
        assert_eq!(left_at(None, Some(12.)), 12.);
        assert_eq!(left_at(None, None), 0.);
    }
    #[::core::prelude::v1::test]
    fn ntsc_projects_get_the_exact_rate_the_viewer_needs() {
        let rate = |json: &str| {
            let doc = pm_model::Document::new(Some(pm_model::json::parse(json).unwrap()), 1);
            composition_rate(doc.root().unwrap())
        };
        assert_eq!(
            rate(r#"{"fps":29.97}"#),
            pm_time::Rate::new(30000, 1001).unwrap()
        );
        assert_eq!(
            rate(r#"{"fps":23.976}"#),
            pm_time::Rate::new(24000, 1001).unwrap()
        );
        assert_eq!(
            rate(r#"{"fps":30,"fpsRational":[60000,1001]}"#),
            pm_time::Rate::new(60000, 1001).unwrap()
        );
        assert_eq!(rate(r#"{"fps":25}"#), pm_time::Rate::new(25, 1).unwrap());
        assert!(rate(r#"{"fps":29.97}"#).exact_frame_ticks().is_some());
    }
    #[::core::prelude::v1::test]
    fn save_note_shows_progress_keeps_refusals_and_reports_the_outcome() {
        assert_eq!(save_note(false, true, true, "Opened"), Some(SAVING));
        assert_eq!(save_note(true, true, true, SAVING), None);
        let refusal = "Saving in progress. Please wait for it to finish before closing.";
        assert_eq!(save_note(false, true, true, refusal), None);
        let fence = "A project operation is in progress. Try closing again when it finishes.";
        assert_eq!(save_note(false, true, true, fence), None);
        assert_eq!(save_note(true, false, false, refusal), Some("Saved"));
        assert_eq!(save_note(true, false, false, fence), Some("Saved"));
        assert_eq!(
            save_note(true, false, true, SAVING),
            Some("Save did not complete; recovery data retained")
        );
        // Other notes set during the save (errors, UI save results) stay.
        assert_eq!(save_note(true, false, false, "Saved"), None);
        assert_eq!(save_note(true, false, false, "Edit failed"), None);
        assert_eq!(save_note(false, false, false, "Opened"), None);
    }
}

#[cfg(test)]
mod open_failure_tests {
    use super::*;
    /// A window's open failure always says what went wrong: never the generic
    /// "operation couldn't be completed" (the codes once missed the wire names).
    #[::core::prelude::v1::test]
    fn window_open_failures_are_worded() {
        for (failure, words) in [
            (KernelFailure::NoProject("p".into()), "can’t be found"),
            (KernelFailure::Journal("disk".into()), "couldn’t be opened"),
            (
                KernelFailure::Storage("Permission denied (os error 13)".into()),
                "permission",
            ),
            (
                KernelFailure::Storage("No such file or directory".into()),
                "can’t be found",
            ),
            (
                KernelFailure::Storage("No space left on device".into()),
                "disk space",
            ),
            (
                KernelFailure::Storage("unexpected end of input".into()),
                "couldn’t be read",
            ),
        ] {
            let alert =
                crate::surfaces::documents::open_error("Film", open_failure(&failure), false);
            assert!(
                !alert.message.contains("operation couldn’t be completed"),
                "{failure:?}: {}",
                alert.message
            );
            assert!(
                alert.message.contains(words),
                "{failure:?}: {}",
                alert.message
            );
        }
    }
}
