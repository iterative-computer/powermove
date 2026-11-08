use crate::{
    layout::{self, Divider, DropPreview, DropTarget, Edge, Geometry, Layout, LayoutStore, Rect},
    native::{Command, Host, MenuEntry, MenuState},
    session::{Session, WindowState},
};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc, time::Duration};

/// A tab drag in progress: the layout it started from, the pointer and the
/// target it would drop on. Escape restores `original`.
#[derive(Clone)]
struct PanelDrag {
    id: String,
    stack: bool,
    start: Point<Pixels>,
    original: Layout,
    at: Point<Pixels>,
    moved: bool,
    preview: Option<DropPreview>,
}
#[derive(Clone)]
enum Gesture {
    Resize(Divider, Point<Pixels>, Layout),
    Panel(PanelDrag),
}
impl Gesture {
    fn original(self) -> Layout {
        match self {
            Gesture::Resize(_, _, l) => l,
            Gesture::Panel(d) => d.original,
        }
    }
}
pub struct Editor {
    pub integration: Option<Rc<dyn crate::integration::Integration>>,
    pub layout: Layout,
    pub tokens: Tokens,
    pub host: Option<Rc<RefCell<Host>>>,
    geometry: Geometry,
    maximized_panel: Option<String>,
    /// Tabs of panels whose owner went away for a moment (an extension
    /// reloading), and where they were: the panel returns there.
    parked: HashMap<String, DropTarget>,
    /// Docking area of the last render, in window points.
    dock_bounds: Rect,
    gesture: Option<Gesture>,
    focus: FocusHandle,
    panel_focus: HashMap<String, FocusHandle>,
    panel_order: Vec<String>,
    /// The panel Ctrl-F6 moved to. Only it shows a focus ring, until a press.
    ringed_panel: Option<String>,
    workspace_focus: FocusHandle,
    pub status: String,
    store: LayoutStore,
    path: Option<PathBuf>,
    project: String,
    split_focus: Option<Divider>,
    /// Session window id; 0 until the window is registered (tests, smoke).
    pub window_id: u64,
    session_path: Option<PathBuf>,
    session_pending: bool,
    tool: String,
}
/// Layout store and window session paths. `PM_NATIVE_SHELL_STATE` selects an
/// isolated layout file; its directory then also holds the session.
pub fn state_paths() -> Option<(PathBuf, PathBuf)> {
    let layouts = std::env::var_os("PM_NATIVE_SHELL_STATE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                PathBuf::from(h)
                    .join("Library/Application Support/Powermove Native/Shell/layouts.json")
            })
        })?;
    let session = layouts.with_file_name("session.json");
    Some((layouts, session))
}
impl Editor {
    pub fn new(tokens: Tokens, host: Option<Rc<RefCell<Host>>>, cx: &mut Context<Self>) -> Self {
        let paths = if host.is_none() { None } else { state_paths() };
        let path = paths.as_ref().map(|p| p.0.clone());
        let store = path
            .as_deref()
            .filter(|p| p.exists())
            .map(LayoutStore::load)
            .transpose();
        let (store, status) = match store {
            Ok(s) => (s.unwrap_or_default(), String::new()),
            Err(e) => {
                eprintln!("Workspace store could not be read: {e}");
                (
                    LayoutStore::default(),
                    "Your workspaces couldn’t be restored, so the default is shown.".into(),
                )
            }
        };
        let project = "shell-untitled".to_string();
        let layout = store.get(
            &project,
            store.active.get(&project).map_or("design", String::as_str),
        );
        Self {
            integration: None,
            layout,
            tokens,
            host,
            geometry: Geometry::default(),
            maximized_panel: None,
            parked: HashMap::new(),
            dock_bounds: Rect::default(),
            gesture: None,
            focus: cx.focus_handle(),
            panel_focus: HashMap::new(),
            ringed_panel: None,
            panel_order: vec![],
            workspace_focus: cx.focus_handle().tab_stop(true).tab_index(2000),
            status,
            store,
            path,
            project,
            split_focus: None,
            window_id: 0,
            session_path: paths.map(|p| p.1),
            session_pending: false,
            tool: "toolSelect".into(),
        }
    }
    /// Binds this editor to a session window and restores its project/workspace.
    pub fn restore(&mut self, state: &WindowState) {
        self.window_id = state.id;
        self.project = state.project.clone();
        self.layout = self.store.get(&self.project, &state.workspace);
    }
    pub fn follow_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::env::var_os("PM_NATIVE_APPEARANCE").is_none() {
            self.tokens = crate::theme::tokens(
                match window.appearance() {
                    WindowAppearance::Dark | WindowAppearance::VibrantDark => {
                        pm_design::Scheme::Dark
                    }
                    _ => pm_design::Scheme::Light,
                },
                crate::native::settings(),
            );
            if let Some(h) = &self.host {
                h.borrow_mut().update_tokens(self.tokens.clone());
            }
            cx.notify();
        }
    }
    /// Title, `Edited`/`Saving…` subtitle (`document_subtitle`) and proxy icon.
    pub fn set_document_metadata(
        &mut self,
        title: &str,
        subtitle: &str,
        edited: bool,
        path: Option<&std::path::Path>,
    ) {
        if let Some(host) = &self.host {
            host.borrow()
                .set_document_metadata(title, subtitle, edited, path);
        }
    }
    /// Applies a theme's `nativePatch` (`Tokens::apply_patch`) to every editor
    /// window; it is re-applied on appearance and accessibility changes.
    pub fn apply_theme(&mut self, patch: crate::theme::Patch, cx: &mut Context<Self>) {
        match crate::theme::set(patch) {
            Ok(()) => self.rebuild_tokens(),
            Err(e) => self.status = format!("The theme could not be applied: {}", e.0),
        }
        cx.notify();
    }
    fn rebuild_tokens(&mut self) {
        self.tokens = crate::theme::tokens(self.tokens.scheme, crate::native::settings());
        if let Some(h) = &self.host {
            h.borrow_mut().update_tokens(self.tokens.clone());
        }
    }
    pub fn bind_project(&mut self, project: String) {
        self.save();
        self.project = project;
        self.layout = self.store.get(
            &self.project,
            self.store
                .active
                .get(&self.project)
                .map_or("design", String::as_str),
        );
        self.persist_session();
    }
    pub fn save(&mut self) {
        if let Some(path) = &self.path
            && path.exists()
        {
            match LayoutStore::load(path) {
                Ok(latest) => self.store = latest,
                Err(e) => {
                    self.status = format!("Layout could not be saved: {e}");
                    return;
                }
            }
        }
        self.store.put(&self.project, self.layout.clone());
        if let Some(p) = &self.path
            && let Err(e) = self.store.save(p)
        {
            self.status = format!("Layout could not be saved: {e}");
        }
        self.persist_session();
    }
    /// This window's restorable state, read from AppKit.
    pub fn window_state(&self) -> Option<WindowState> {
        let host = self.host.as_ref()?.borrow();
        Some(WindowState {
            id: self.window_id,
            frame: host.frame(),
            project: self.project.clone(),
            workspace: self.layout.workspace.clone(),
            tab_group: host.tab_leader(),
            fullscreen: host.is_fullscreen(),
        })
    }
    /// Read-modify-write of the shared session file (other windows write too).
    /// A malformed or newer file is reported and never overwritten.
    pub fn persist_session(&mut self) {
        if self.window_id == 0 {
            return;
        }
        let (Some(path), Some(state)) = (self.session_path.clone(), self.window_state()) else {
            return;
        };
        let key = self.host.as_ref().is_some_and(|h| h.borrow().is_key());
        let id = self.window_id;
        self.update_session(&path, |s| {
            s.upsert(state);
            if key {
                s.key = Some(id);
            }
        });
    }
    /// Removes this window from the session (the user closed it).
    pub fn forget_session(&mut self) {
        if let Some(path) = self.session_path.clone()
            && self.window_id != 0
        {
            let id = self.window_id;
            self.update_session(&path, |s| s.remove(id));
        }
    }
    fn update_session(&mut self, path: &std::path::Path, f: impl FnOnce(&mut Session)) {
        let mut session = if path.exists() {
            match Session::load(path) {
                Ok(s) => s,
                Err(e) => {
                    self.status = format!("Window session could not be saved: {e}");
                    return;
                }
            }
        } else {
            Session::default()
        };
        f(&mut session);
        session.version = 1;
        if let Err(e) = session.save(path) {
            self.status = format!("Window session could not be saved: {e}");
        }
    }
    /// Coalesces live-resize/move notifications into one session write.
    pub fn schedule_session(&mut self, cx: &mut Context<Self>) {
        if self.session_pending || self.window_id == 0 {
            return;
        }
        self.session_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let _ = this.update(cx, |v, _| {
                v.session_pending = false;
                v.persist_session();
            });
        })
        .detach();
    }
    fn menu_state(&self, cx: &App) -> MenuState {
        MenuState {
            open: self
                .layout
                .open_panels()
                .into_iter()
                .map(String::from)
                .collect(),
            saved: self.store.saved.keys().cloned().collect(),
            workspace: self.layout.workspace.clone(),
            sidebar: self.dock_visible("left"),
            inspector: self.dock_visible("right"),
            tool: self.tool.clone(),
            disabled: self
                .integration
                .as_ref()
                .map(|i| i.disabled(cx))
                .unwrap_or_default(),
        }
    }
    fn dock_visible(&self, id: &str) -> bool {
        self.layout
            .docks
            .iter()
            .any(|d| d.id == id && !d.collapsed && !d.groups.is_empty())
    }
    /// Shell-owned commands. Returns false when the command is not the shell's.
    fn shell_command(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let mut parts = name.splitn(3, ':');
        let (Some(kind), Some(verb)) = (parts.next(), parts.next()) else {
            return false;
        };
        let rest = parts.next().unwrap_or("");
        let result = match (kind, verb) {
            ("panel", "toggle") => {
                if self.layout.open_panels().contains(&rest) && rest != "viewer" {
                    self.layout.close_panel(rest)
                } else {
                    self.layout.show_panel(rest)
                }
            }
            ("panel", "activate") => {
                // Switching a tab preserves the maximized stack.
                let result = self.layout.show_panel(rest);
                if let Err(error) = result {
                    self.status = error.to_string();
                } else {
                    self.save();
                }
                cx.notify();
                return true;
            }
            ("panel", "show") => match self.parked.remove(rest) {
                Some(place) if self.layout.find(rest).is_none() => {
                    let mut next = self.layout.clone();
                    next.show_panel(rest)
                        .and_then(|_| next.move_panel_to(rest, place))
                        .map(|_| self.layout = next)
                        .or_else(|_| self.layout.show_panel(rest))
                }
                _ => self.layout.show_panel(rest),
            },
            ("panel", "close") => {
                self.parked.remove(rest);
                self.layout.close_panel(rest)
            }
            // panel:park:<id> — closed, but a later show puts it back in place.
            ("panel", "park") => match self.layout.find(rest) {
                Some((dock, group, index)) => {
                    // A tab alone in its stack takes the stack with it; it
                    // comes back as a stack in the same position.
                    let alone = self.layout.docks[dock].groups[group].tabs.len() == 1;
                    let place = if alone {
                        DropTarget::Split { dock, group }
                    } else {
                        DropTarget::Tab { dock, group, index }
                    };
                    self.parked.insert(rest.to_owned(), place);
                    self.layout.close_panel(rest)
                }
                None => Ok(()),
            },
            ("panel", "dock") => {
                let (id, edge) = rest.rsplit_once(':').unwrap_or((rest, ""));
                let edge = match edge {
                    "left" => Edge::Left,
                    "right" => Edge::Right,
                    _ => Edge::Bottom,
                };
                self.layout.move_panel_to(id, DropTarget::Edge(edge))
            }
            ("panel", "group" | "above" | "below") => {
                let (id, target) = rest.split_once(':').unwrap_or((rest, ""));
                self.layout.place_relative(id, target, verb)
            }
            ("panel", "maximize") => {
                if self.layout.open_panels().contains(&rest) {
                    self.maximized_panel = Some(rest.into());
                    cx.notify();
                }
                return true;
            }
            // panel:addto:<dock>:<group>:<id> — the header's + menu.
            ("panel", "addto") => {
                let mut it = rest.splitn(3, ':');
                let dock = it.next().and_then(|s| s.parse().ok());
                let group = it.next().and_then(|s| s.parse().ok());
                let id = it.next().unwrap_or("");
                match (dock, group) {
                    (Some(dock), Some(group)) => {
                        let mut next = self.layout.clone();
                        next.show_panel(id)
                            .and_then(|_| {
                                next.move_panel_to(
                                    id,
                                    DropTarget::Tab {
                                        dock,
                                        group,
                                        index: usize::MAX,
                                    },
                                )
                            })
                            .map(|_| self.layout = next)
                    }
                    _ => Err(anyhow::anyhow!("Unknown group")),
                }
            }
            ("layout", "save") if rest.is_empty() => {
                if let Some(h) = &self.host {
                    h.borrow().prompt_workspace_name(&self.layout.workspace);
                }
                Ok(())
            }
            ("layout", "save") => {
                let layout = self.layout.clone();
                let mut saved = String::new();
                self.mutate_store(|s| s.save_named(rest, &layout).map(|n| saved = n))
                    .map(|_| self.status = format!("Saved workspace “{saved}”"))
            }
            ("layout", "apply") => self
                .store
                .named(rest, &self.layout.workspace)
                .map(|l| self.layout = l)
                .ok_or_else(|| anyhow::anyhow!("There’s no workspace named “{rest}”.")),
            ("layout", "delete") => self.mutate_store(|s| {
                s.delete_named(rest);
                Ok(())
            }),
            ("layout", "reset") => {
                self.layout = Layout::preset(&self.layout.workspace);
                Ok(())
            }
            _ => return false,
        };
        match result {
            Ok(()) => {
                self.maximized_panel = None;
                self.save();
            }
            Err(e) => self.status = e.to_string(),
        }
        cx.notify();
        true
    }
    /// Reloads the latest store (other windows write it), applies `f` and writes
    /// it back atomically. A malformed/newer store is reported, never replaced.
    fn mutate_store(
        &mut self,
        f: impl FnOnce(&mut LayoutStore) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        if let Some(p) = self.path.clone().filter(|p| p.exists()) {
            self.store = LayoutStore::load(&p)?;
        }
        f(&mut self.store)?;
        if let Some(p) = &self.path {
            self.store.version = 1;
            self.store.save(p)?;
        }
        Ok(())
    }
    pub fn command(&mut self, cmd: Command, window: &mut Window, cx: &mut Context<Self>) {
        if cmd.name == "shellReplayComplete" {
            if let (Some(token), Some(host)) = (cmd.token, &self.host) {
                host.borrow_mut().replayed(token);
            }
            return;
        }
        if matches!(
            cmd.name.as_str(),
            "shellFocusNext" | "shellFocusPrevious" | "shellPanelNext" | "shellPanelPrevious"
        ) {
            let reverse = cmd.name.ends_with("Previous");
            if cmd.name.starts_with("shellPanel") {
                let current = self
                    .panel_order
                    .iter()
                    .position(|id| self.panel_focus[id].contains_focused(window, cx));
                if !self.panel_order.is_empty() {
                    let n = self.panel_order.len();
                    let index = current.map_or(if reverse { n - 1 } else { 0 }, |i| {
                        if reverse {
                            (i + n - 1) % n
                        } else {
                            (i + 1) % n
                        }
                    });
                    window.focus(&self.panel_focus[&self.panel_order[index]], cx);
                    self.ringed_panel = Some(self.panel_order[index].clone());
                }
            } else {
                let first = self.panel_order.first().map(|id| &self.panel_focus[id]);
                let from_toolbar = cmd.args.first().and_then(|v| v.as_bool()) == Some(true);
                let boundary = window.focused(cx).is_none()
                    || if reverse {
                        first.is_some_and(|h| h.is_focused(window))
                    } else {
                        self.workspace_focus.is_focused(window)
                    };
                if boundary
                    && !from_toolbar
                    && self
                        .host
                        .as_ref()
                        .is_some_and(|h| h.borrow().focus_toolbar(reverse))
                {
                    // Native toolbar focus must not leave a second GPUI ring behind.
                    window.blur(cx);
                    cx.notify();
                    return;
                }
                if from_toolbar {
                    if reverse {
                        window.focus(&self.workspace_focus, cx);
                    } else if let Some(first) = first {
                        window.focus(first, cx);
                    }
                } else if reverse {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
            }
            self.split_focus = None;
            if let Some(h) = &self.host {
                h.borrow_mut().splitter_focus = false;
            }
            cx.notify();
            return;
        }
        eprintln!(
            "shell command={} args={:?} token={:?}",
            cmd.name, cmd.args, cmd.token
        );
        // A command that cannot run declines, so its key reaches the focused
        // control and no route ever does nothing silently.
        if self
            .integration
            .as_ref()
            .is_some_and(|i| i.disabled(cx).contains(&cmd.name))
        {
            if let (Some(token), Some(host)) = (cmd.token, &self.host) {
                host.borrow_mut().resolve(token, false);
            }
            return;
        }
        if cmd.name.starts_with("tool") && cmd.name.len() > 4 {
            // Keep the toolbar's tool group in step with keyboard/menu selection.
            self.tool = cmd.name.clone();
            if let Some(h) = &self.host {
                h.borrow().select_tool(&cmd.name);
            }
        }
        if let Some(host) = &self.host {
            host.borrow().canvas_focus.set(
                window.focused(cx).is_none_or(|f| {
                    f == self.focus || self.panel_focus.get("viewer").is_some_and(|p| *p == f)
                }) && self.layout.open_panels().contains(&"viewer"),
            );
        }
        if self
            .integration
            .clone()
            .is_some_and(|i| i.command(&cmd, window, cx))
        {
            if let (Some(token), Some(host)) = (cmd.token, &self.host) {
                host.borrow_mut().resolve(token, true);
            }
            cx.notify();
            return;
        }
        if cmd.name.starts_with("shellTest:") {
            self.test_command(&cmd, cx);
            return;
        }
        let mut handled = true;
        match cmd.name.as_str() {
            "palette" => {
                if let Some(h) = &self.host {
                    let state = self.menu_state(cx);
                    h.borrow_mut().open_palette(&state);
                }
            }
            "fieldMenu" => {
                if let Some(h) = &self.host {
                    h.borrow_mut().popup(true);
                }
            }
            "maximizePanel" => {
                if self.maximized_panel.take().is_none() {
                    let at = window.mouse_position();
                    self.maximized_panel = self
                        .geometry
                        .panels
                        .iter()
                        .find(|p| p.rect.contains(at.x.into(), at.y.into()))
                        .map(|p| {
                            self.layout.docks[p.dock].groups[p.group]
                                .active()
                                .to_owned()
                        })
                        .or_else(|| {
                            self.panel_order
                                .iter()
                                .find(|id| {
                                    self.panel_focus
                                        .get(*id)
                                        .is_some_and(|f| f.is_focused(window))
                                })
                                .cloned()
                        });
                    handled = self.maximized_panel.is_some();
                }
            }
            "restorePanel" => {
                self.maximized_panel = None;
            }
            "toggleSidebar" | "toggleInspector" => {
                self.maximized_panel = None;
                let id = if cmd.name == "toggleSidebar" {
                    "left"
                } else {
                    "right"
                };
                if let Some(d) = self.layout.docks.iter_mut().find(|d| d.id == id) {
                    d.collapsed = !d.collapsed;
                    self.save();
                }
            }
            "shellAppearanceChanged" => self.rebuild_tokens(),
            // A theme from the theme pipeline: args[0] is its JSON output.
            "shellTheme" => {
                let json = cmd.args.first().map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    v => v.to_string(),
                });
                match json.as_deref().map(crate::theme::parse) {
                    Some(Ok(patch)) => self.apply_theme(patch, cx),
                    Some(Err(e)) => {
                        self.status = format!("The theme could not be applied: {}", e.0)
                    }
                    None => {
                        crate::theme::clear();
                        self.rebuild_tokens();
                    }
                }
            }
            "settings" => crate::open_settings(cx),
            "newWindow" => crate::open_editor(cx),
            "newTab" => crate::open_editor_tab(cx, self.window_id),
            "deselect" | "cancel" => {
                if let Some(g) = self.gesture.take() {
                    self.layout = g.original();
                } else {
                    handled = false;
                }
            }
            name if name.starts_with("workspace:") => {
                self.maximized_panel = None;
                self.save();
                self.layout = self.store.get(&self.project, &name[10..]);
                self.save();
            }
            name if name.starts_with("tool") && name.len() > 4 => {}
            name if self.shell_command(name, cx) => {}
            name => {
                handled = self.host.as_ref().is_some_and(|h| h.borrow().system(name));
                if !handled {
                    eprintln!("shell: unhandled command {name}");
                }
            }
        }
        if let (Some(token), Some(host)) = (cmd.token, &self.host) {
            host.borrow_mut().resolve(token, handled);
        }
        cx.notify();
    }
    /// Screen-free stand-in for a pointer drag of panel `id` to `(x, y)`, using the
    /// last rendered geometry. Used by headless captures and tests.
    #[doc(hidden)]
    pub fn simulate_drag(&mut self, id: &str, x: f32, y: f32) {
        let at = point(px(x), px(y));
        let original = match self.gesture.take() {
            Some(Gesture::Panel(d)) => d.original,
            _ => self.layout.clone(),
        };
        let preview = self.layout.drop_target_for(
            &self.geometry,
            self.dock_bounds,
            (x, y),
            &self.tokens,
            id,
            false,
        );
        self.gesture = Some(Gesture::Panel(PanelDrag {
            id: id.into(),
            stack: false,
            start: at,
            original,
            at,
            moved: true,
            preview,
        }));
    }
    /// Completes a simulated drag like a mouse-up; returns the applied target.
    #[doc(hidden)]
    pub fn simulate_drop(&mut self) -> Option<DropTarget> {
        let Some(Gesture::Panel(drag)) = self.gesture.take() else {
            return None;
        };
        let target = drag.preview?.target;
        if drag.stack {
            self.layout.move_stack_to(&drag.id, target)
        } else {
            self.layout.move_panel_to(&drag.id, target)
        }
        .ok()?;
        self.save();
        Some(target)
    }
    fn begin_panel_drag(&mut self, id: &str, at: Point<Pixels>, stack: bool) {
        self.maximized_panel = None;
        self.gesture = Some(Gesture::Panel(PanelDrag {
            id: id.into(),
            stack,
            start: at,
            original: self.layout.clone(),
            at,
            moved: false,
            preview: None,
        }));
        self.split_focus = None;
        if let Some(h) = &self.host {
            h.borrow_mut().splitter_focus = false;
        }
    }
    /// Test-driver commands (`PM_PERF_GATE=1` runs only): drag a tab to a point,
    /// drop it, open a tab's context menu, close transients.
    fn test_command(&mut self, cmd: &Command, cx: &mut Context<Self>) {
        if std::env::var("PM_PERF_GATE").as_deref() != Ok("1") {
            return;
        }
        let s = |i: usize| {
            cmd.args
                .get(i)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let f = |i: usize| cmd.args.get(i).and_then(|v| v.as_f64()).unwrap_or(0.) as f32;
        match &cmd.name["shellTest:".len()..] {
            "drag" => self.simulate_drag(&s(0), f(1), f(2)),
            "drop" => {
                self.simulate_drop();
            }
            "cancelDrag" => {
                if let Some(g) = self.gesture.take() {
                    self.layout = g.original();
                }
            }
            "tabMenu" => self.tab_menu(&s(0), point(px(f(1)), px(f(2))), cx),
            "dismiss" => {
                if let Some(h) = &self.host {
                    h.borrow().dismiss_transients();
                }
            }
            _ => {}
        }
        cx.notify();
    }
    /// Swaps the glass materials for their Reduce Transparency fallback (on)
    /// and back (off), so a panel capture never shows what's behind the window.
    /// Returns the tokens to restore.
    pub fn solid_materials(&mut self, restore: Option<Tokens>) -> Option<Tokens> {
        match restore {
            Some(tokens) => {
                self.tokens = tokens;
                None
            }
            None => {
                let solid = Tokens::new(
                    self.tokens.scheme,
                    pm_design::Accessibility {
                        reduce_transparency: true,
                        ..self.tokens.accessibility
                    },
                );
                let previous = self.tokens.clone();
                self.tokens.translucency = solid.translucency;
                self.tokens.accessibility.reduce_transparency = true;
                Some(previous)
            }
        }
    }
    /// The window-point rect of the stack showing panel `id` (tab strip
    /// included) in the last layout pass; `None` when it isn't on screen.
    pub fn panel_rect(&self, id: &str) -> Option<Rect> {
        self.geometry
            .panels
            .iter()
            .find(|p| {
                let group = &self.layout.docks[p.dock].groups[p.group];
                group.tabs.get(group.active).is_some_and(|tab| tab == id)
                    && self
                        .maximized_panel
                        .as_deref()
                        .is_none_or(|maximized| maximized == id)
            })
            .map(|p| p.rect)
    }
    /// The live arrangement in the agents' digest shape (get_panel_layout).
    pub fn layout_digest(&self) -> serde_json::Value {
        let docks: Vec<_> = self
            .layout
            .docks
            .iter()
            .map(|dock| {
                let panels: Vec<_> = dock
                    .groups
                    .iter()
                    .flat_map(|group| {
                        group.tabs.iter().enumerate().map(move |(i, id)| {
                            serde_json::json!({"id":id,"title":crate::layout::panel_title(id),
                                "collapsed":dock.collapsed || i != group.active,
                                "flex":group.size.is_none(),"size":group.size})
                        })
                    })
                    .collect();
                serde_json::json!({"id":dock.id,"size":dock.size,"panels":panels})
            })
            .collect();
        let name = crate::layout::PRESETS
            .iter()
            .find(|(id, _)| *id == self.layout.workspace)
            .map_or(self.layout.workspace.as_str(), |(_, name)| name);
        serde_json::json!({"workspace":{"id":self.layout.workspace,"name":name},"docks":docks})
    }
    /// What set_panel_layout's background write needs: the layout file, this
    /// window's project key, and `docks` as a layout of this workspace.
    pub fn agent_layout_plan(
        &self,
        docks: Vec<crate::layout::Dock>,
    ) -> (Option<PathBuf>, String, Layout) {
        let layout = Layout {
            version: self.layout.version,
            workspace: self.layout.workspace.clone(),
            docks,
        };
        (self.path.clone(), self.project.clone(), layout)
    }
    /// Shows a layout whose store write already landed (no disk access here).
    pub fn adopt_agent_layout(
        &mut self,
        store: LayoutStore,
        layout: Layout,
        cx: &mut Context<Self>,
    ) {
        self.store = store;
        self.layout = layout;
        self.maximized_panel = None;
        cx.notify();
    }
    /// Runs a shell command without an AppKit host (tests).
    #[doc(hidden)]
    pub fn run_shell_command(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        self.shell_command(name, cx)
    }
    /// Context menu for a panel tab (also its header). The panel's current
    /// dock carries a checkmark.
    pub fn tab_menu_entries(&self, id: &str) -> Vec<MenuEntry> {
        let title = layout::panel_title(id);
        let current = self
            .layout
            .docks
            .iter()
            .find(|d| d.groups.iter().any(|g| g.tabs.iter().any(|t| t == id)))
            .map(|d| d.id.as_str());
        let mut items = vec![];
        if id != "viewer" {
            items.push(MenuEntry::item(
                format!("Close {title}"),
                format!("panel:close:{id}"),
            ));
            items.push(MenuEntry::separator());
        }
        for (label, edge) in [
            ("Dock Left", "left"),
            ("Dock Right", "right"),
            ("Dock Bottom", "bottom"),
        ] {
            items.push(
                MenuEntry::item(label, format!("panel:dock:{id}:{edge}"))
                    .checked(current == Some(edge)),
            );
        }
        for (label, verb) in [
            ("Group With", "group"),
            ("Dock Above", "above"),
            ("Dock Below", "below"),
        ] {
            let children = self
                .layout
                .open_panels()
                .into_iter()
                .filter(|target| {
                    *target != id && (verb != "group" || (id != "viewer" && *target != "viewer"))
                })
                .map(|target| {
                    MenuEntry::item(
                        layout::panel_title(target),
                        format!("panel:{verb}:{id}:{target}"),
                    )
                })
                .collect();
            items.push(MenuEntry::submenu(label, children));
        }
        items.push(MenuEntry::item(
            "Maximize Panel",
            format!("panel:maximize:{id}"),
        ));
        items.push(MenuEntry::separator());
        items.push(MenuEntry::item("Save Workspace…", "layout:save"));
        items.push(MenuEntry::item("Reset Workspace", "layout:reset"));
        items
    }
    fn tab_menu(&self, id: &str, position: Point<Pixels>, cx: &mut App) {
        let Some(h) = &self.host else { return };
        let at = (f32::from(position.x), f32::from(position.y));
        let entries = self.tab_menu_entries(id);
        let entries = match &self.integration {
            Some(integration) => integration.panel_menu(id, entries, at, cx),
            None => Some(entries),
        };
        if let Some(entries) = entries {
            h.borrow().context_menu(&entries, at.0, at.1);
        }
    }
    /// The status bar's workspace pop-up: presets (current checked) and saved ones.
    fn workspace_menu(&self, position: Point<Pixels>) {
        let Some(h) = &self.host else { return };
        let mut items: Vec<_> = layout::PRESETS
            .iter()
            .map(|(id, label)| {
                MenuEntry::item(*label, format!("workspace:{id}"))
                    .checked(*id == self.layout.workspace)
            })
            .collect();
        if !self.store.saved.is_empty() {
            items.push(MenuEntry::separator());
            items.extend(
                self.store
                    .saved
                    .keys()
                    .map(|n| MenuEntry::item(n.clone(), format!("layout:apply:{n}"))),
            );
        }
        items.push(MenuEntry::separator());
        items.push(MenuEntry::item("Save Workspace…", "layout:save"));
        h.borrow()
            .context_menu(&items, f32::from(position.x), f32::from(position.y));
    }
    fn add_menu(&self, dock: usize, group: usize, position: Point<Pixels>) {
        let Some(h) = &self.host else { return };
        let open = self.layout.open_panels();
        let items: Vec<_> = layout::all_panels()
            .iter()
            .filter(|p| !open.contains(&p.as_str()))
            .map(|p| {
                MenuEntry::item(
                    layout::panel_title(p),
                    format!("panel:addto:{dock}:{group}:{p}"),
                )
            })
            .collect();
        if items.is_empty() {
            return;
        }
        h.borrow()
            .context_menu(&items, f32::from(position.x), f32::from(position.y));
    }
}
/// A token colour at an alpha (`translucency.*`, `Tokens::edge`, `Tokens::line`).
pub fn tint(rgb: u32, alpha: f32) -> gpui::Rgba {
    gpui::rgba((rgb << 8) | (alpha.clamp(0., 1.) * 255.).round() as u32)
}
fn positioned(r: Rect) -> Div {
    div()
        .absolute()
        .left(px(r.x))
        .top(px(r.y))
        .w(px(r.w))
        .h(px(r.h))
}
/// Centred empty state (language §9/§14): `heading` + one `reading` line.
/// Callers may append one action.
pub fn empty_state(t: &Tokens, heading: &str, line: &str) -> Div {
    let (c, ty) = (&t.colors, t.typography);
    div()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(t.metrics.border * 2.))
        .px(px(t.metrics.inset))
        .child(
            div()
                .text_size(px(ty.heading))
                .font_weight(FontWeight(ty.semibold_weight))
                .text_color(rgb(c.secondary))
                .child(heading.to_string()),
        )
        .child(
            div()
                .text_size(px(ty.reading))
                .text_color(rgb(c.muted))
                .text_center()
                .child(line.to_string()),
        )
}
struct Tip {
    text: SharedString,
    tokens: Tokens,
}
impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (c, m) = (&self.tokens.colors, self.tokens.metrics);
        div()
            .px(px(m.gap))
            .py(px(m.border * 2.))
            .rounded(px(m.control_radius))
            .bg(rgb(c.floating))
            .border(px(m.border))
            .border_color(rgb(c.separator))
            .text_size(px(self.tokens.typography.minimum))
            .text_color(rgb(c.text))
            .font_family(pm_design::TEXT_FONT)
            .child(self.text.clone())
    }
}
/// A tooltip builder naming a control's action.
pub fn tooltip(
    t: &Tokens,
    text: &'static str,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let tokens = t.clone();
    move |_, cx| {
        cx.new(|_| Tip {
            text: text.into(),
            tokens: tokens.clone(),
        })
        .into()
    }
}
impl Render for Editor {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(id) = self.integration.as_ref().and_then(|i| i.project_id(cx))
            && self.project != id
        {
            self.bind_project(id);
        }
        let t = self.tokens.clone();
        let m = t.metrics;
        let c = t.colors.clone();
        let viewport = w.viewport_size();
        let top = self
            .host
            .as_ref()
            .map_or(m.toolbar, |h| h.borrow().toolbar_inset());
        self.dock_bounds = Rect {
            x: m.gap,
            y: top + m.gap,
            w: (f32::from(viewport.width) - m.gap * 2.).max(0.),
            h: (f32::from(viewport.height) - top - m.status - m.gap * 2.).max(0.),
        };
        self.geometry = self.layout.geometry_with_maximized(
            self.dock_bounds,
            &t,
            self.maximized_panel.as_deref(),
        );
        let mut ordered = self.geometry.panels.clone();
        ordered.sort_by(|a, b| {
            a.rect
                .y
                .total_cmp(&b.rect.y)
                .then(a.rect.x.total_cmp(&b.rect.x))
        });
        self.panel_order = ordered
            .iter()
            .map(|p| {
                self.layout.docks[p.dock].groups[p.group]
                    .active()
                    .to_string()
            })
            .collect();
        for id in &self.panel_order {
            self.panel_focus
                .entry(id.clone())
                .or_insert_with(|| cx.focus_handle().tab_stop(true).tab_index(0));
        }
        let viewer = self
            .geometry
            .panels
            .iter()
            .find(|p| self.layout.docks[p.dock].groups[p.group].active() == "viewer")
            .map(|p| Rect {
                x: p.rect.x + m.border,
                y: p.rect.y + m.header,
                w: (p.rect.w - m.border * 2.).max(0.),
                h: (p.rect.h - m.header - m.border).max(0.),
            });
        if let Some(h) = &self.host {
            let h = h.borrow();
            h.viewer_rect.set(viewer);
            h.canvas_focus.set(
                viewer.is_some()
                    && w.focused(cx).is_none_or(|f| {
                        f == self.focus || self.panel_focus.get("viewer").is_some_and(|p| *p == f)
                    }),
            );
            h.publish_menu_state(&self.menu_state(cx));
            h.keyboard_control_focus.set(
                w.focused(cx).is_some_and(|f| {
                    f != self.focus && !self.panel_focus.values().any(|p| *p == f)
                }),
            );
        }
        let native = self.host.is_some();
        let canvas_viewer = viewer;
        let (ground, ground_alpha) = t.ground();
        let (edge, edge_alpha) = t.edge();
        let (line, line_alpha) = t.line();
        // Extension pages sit below GPUI too; their bodies are read at paint
        // time, after this frame's layout has recorded them.
        let native_bodies = self.host.as_ref().map(|h| h.borrow().native_panels.clone());
        let dragging = matches!(&self.gesture, Some(Gesture::Panel(d)) if d.moved);
        let mut root = div()
            .id("editor")
            .role(Role::Group)
            .accessibility_id("shell.editor")
            .aria_label("Powermove editor")
            .size_full()
            .font_family(pm_design::TEXT_FONT)
            .text_size(px(t.typography.body))
            .text_color(rgb(c.text))
            .track_focus(&self.focus)
            .when(dragging, |d| d.cursor(CursorStyle::ClosedHand))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if let Some(i) = &this.integration {
                    i.drop_paths(paths.paths().to_vec(), cx);
                }
            }))
            .child(
                canvas(
                    move |_, _, _| (),
                    move |b, _, window, _| {
                        let (width, height) = (f32::from(b.size.width), f32::from(b.size.height));
                        let mut holes: Vec<Rect> =
                            canvas_viewer.filter(|_| native).into_iter().collect();
                        if let Some(bodies) = &native_bodies {
                            holes.extend(bodies.borrow().iter().map(|(_, r)| *r));
                        }
                        let rects = layout::outside(
                            Rect {
                                x: 0.,
                                y: 0.,
                                w: width,
                                h: height,
                            },
                            &holes,
                        );
                        for r in rects {
                            window.paint_quad(
                                PaintQuad::new(Bounds::new(
                                    point(px(r.x), px(r.y)),
                                    gpui::size(px(r.w), px(r.h)),
                                ))
                                .background(tint(ground, ground_alpha)),
                            );
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_move_all({
                let listener = cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                    if !e.dragging() {
                        return;
                    }
                    match &mut this.gesture {
                        Some(Gesture::Resize(d, start, original)) => {
                            this.layout = original.clone();
                            let delta = if d.horizontal {
                                f32::from(e.position.y - start.y)
                            } else {
                                f32::from(e.position.x - start.x)
                            };
                            this.layout.resize(d, delta, &this.tokens);
                            cx.notify();
                        }
                        Some(Gesture::Panel(drag)) => {
                            drag.at = e.position;
                            let dx = f32::from(e.position.x - drag.start.x);
                            let dy = f32::from(e.position.y - drag.start.y);
                            drag.moved |= dx.hypot(dy) >= this.tokens.metrics.drag_threshold;
                            drag.preview = drag
                                .moved
                                .then(|| {
                                    this.layout.drop_target_for(
                                        &this.geometry,
                                        this.dock_bounds,
                                        (f32::from(e.position.x), f32::from(e.position.y)),
                                        &this.tokens,
                                        &drag.id,
                                        drag.stack,
                                    )
                                })
                                .flatten();
                            cx.notify();
                        }
                        None => {}
                    }
                });
                move |e, phase, _, w, cx| {
                    if phase == DispatchPhase::Bubble {
                        listener(e, w, cx);
                    }
                }
            })
            .on_mouse_up_all({
                let listener = cx.listener(|this, _: &MouseUpEvent, _, cx| {
                    if let Some(g) = this.gesture.take() {
                        if let Gesture::Panel(drag) = g
                            && drag.moved
                            && let Some(p) = drag.preview
                            && let Err(err) = if drag.stack {
                                this.layout.move_stack_to(&drag.id, p.target)
                            } else {
                                this.layout.move_panel_to(&drag.id, p.target)
                            }
                        {
                            this.status = err.to_string();
                        }
                        this.save();
                        cx.notify();
                    }
                });
                move |e: &MouseUpEvent, phase, _, w, cx| {
                    if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                        listener(e, w, cx);
                    }
                }
            })
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                if this.ringed_panel.take().is_some() {
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    if let Some(g) = this.gesture.take() {
                        this.layout = g.original();
                        cx.notify();
                    }
                    return;
                }
                if let Some(d) = this.split_focus.clone() {
                    let key = e.keystroke.key.as_str();
                    let delta = match key {
                        "left" | "up" => -this.tokens.metrics.keyboard_step,
                        "right" | "down" => this.tokens.metrics.keyboard_step,
                        "enter" => {
                            if d.group.is_none() {
                                let dock = &mut this.layout.docks[d.dock];
                                dock.collapsed = !dock.collapsed;
                            }
                            0.
                        }
                        _ => return,
                    };
                    this.layout.resize(&d, delta, &this.tokens);
                    this.save();
                    cx.notify();
                }
            }));
        if self.host.is_none() {
            root = root.child(
                positioned(Rect {
                    x: m.inset,
                    y: 0.,
                    w: f32::from(viewport.width) - m.inset * 2.,
                    h: m.toolbar,
                })
                .flex()
                .items_center()
                .child("Untitled"),
            );
        }
        let dragged = match &self.gesture {
            Some(Gesture::Panel(d)) if d.moved => Some(d.id.clone()),
            _ => None,
        };
        if let Some(integration) = &self.integration {
            integration.open_panels(&self.layout.open_panels(), cx);
        }
        let mut native_bodies = vec![];
        for p in ordered {
            let group = &self.layout.docks[p.dock].groups[p.group];
            let id = group.active().to_string();
            let is_viewer = id == "viewer";
            let body_native = self
                .integration
                .as_ref()
                .is_some_and(|i| i.native_panel(&id, cx));
            let tabs = group.tabs.clone();
            let active = group.active;
            // The tab strip's own padding (2 pt each side) and 2 pt gaps come
            // out of the width before it is shared between the tabs.
            let strip_chrome = 4. + 2. * tabs.len().saturating_sub(1) as f32;
            let tab_track =
                group.tab_track(p.rect.w - m.inset - m.gap * 2. - m.hit - strip_chrome, &t);
            let mut panel = positioned(p.rect)
                .id(format!("group-{}-{}", p.dock, p.group))
                .role(Role::Group)
                .tab_group()
                .tab_index(self.panel_order.iter().position(|v| v == &id).unwrap_or(0) as isize)
                .track_focus(&self.panel_focus[&id])
                // Keyboard panel focus only (Ctrl-F6): a click never rings a panel.
                .when(self.ringed_panel.as_deref() == Some(id.as_str()), |p| {
                    p.focus_style(|s| s.ring(px(m.border * 2.)).ring_color(rgb(c.focus)))
                })
                .accessibility_id(format!("shell.panel.{id}"))
                .aria_label(layout::panel_title(&id).to_string())
                .rounded(px(m.panel_radius))
                .border(px(m.border))
                .border_color(tint(edge, edge_alpha))
                .overflow_hidden();
            if body_native {
                native_bodies.push((
                    id.to_string(),
                    Rect {
                        x: p.rect.x + m.border,
                        y: p.rect.y + m.header,
                        w: (p.rect.w - m.border * 2.).max(0.),
                        h: (p.rect.h - m.header - m.border).max(0.),
                    },
                ));
            } else if !is_viewer || !native {
                panel = panel.bg(if is_viewer {
                    rgb(c.viewer)
                } else {
                    tint(c.panel, t.translucency.panel)
                });
            }
            // A panel may supply its own header content (viewer: pop-ups, no
            // title; timeline: transport). The shell keeps the tab strip only
            // when the group has more than one tab.
            let custom = self
                .integration
                .as_ref()
                .and_then(|i| i.header(&id, p.rect, w, cx));
            let group_name: SharedString = format!("tabs-{}-{}", p.dock, p.group).into();
            let header_id = id.clone();
            let menu_header = id.clone();
            let mut header = div()
                .id(format!("header-{}-{}", p.dock, p.group))
                .role(Role::Group)
                .aria_label(format!("{} header", layout::panel_title(&id)))
                .group(group_name.clone())
                .flex()
                .items_center()
                .gap(px(m.gap))
                .h(px(m.header))
                .flex_none()
                .pl(px(m.inset))
                .pr(px(m.gap))
                // The native viewer body has no panel fill; its bar carries the material.
                .when(is_viewer && native, |h| {
                    h.bg(tint(c.panel, t.translucency.panel))
                })
                .when(!is_viewer, |h| {
                    h.border_b(px(m.border))
                        .border_color(tint(line, line_alpha))
                })
                // The header's empty area moves the whole stack; a tab moves one panel.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                        this.begin_panel_drag(&header_id, e.position, true);
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                        this.tab_menu(&menu_header, e.position, cx);
                    }),
                );
            let multi = tabs.len() > 1;
            if custom.is_some() && !multi {
                header = header.px(px(0.));
            }
            if multi || custom.is_none() {
                // Approved mockup: a sunken segmented strip for tab groups, a
                // semibold title for single panels. The whole tab is the drag handle.
                // Tabs share what the panel's own header controls leave.
                let mut strip = div().flex().items_center().gap(px(2.)).when(multi, |s| {
                    s.flex_1()
                        .min_w(px(0.))
                        .bg(tint(line, line_alpha))
                        .rounded(px(m.control_radius + 2.))
                        .p(px(2.))
                });
                for (ti, tab) in tabs.iter().enumerate() {
                    if !tab_track.visible.contains(&ti) {
                        continue;
                    }
                    let tabid = tab.clone();
                    let menu_id = tab.clone();
                    let selected = ti == active;
                    let ghost = dragged.as_deref() == Some(tab.as_str());
                    let close_id = tab.clone();
                    let closable = tab != "viewer" && multi;
                    let tab_group: SharedString = format!("tab-{tab}").into();
                    let title = layout::panel_title(tab).to_string();
                    let mut cell = div()
                        .id(format!("tab-{tab}"))
                        .group(tab_group.clone())
                        .role(Role::Tab)
                        .accessibility_id(format!("shell.tab.{tab}"))
                        .aria_selected(selected)
                        .tab_index(0)
                        .focus_style(|s| s.ring(px(m.border * 2.)).ring_color(rgb(c.focus)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.layout.docks[p.dock].groups[p.group].active = ti;
                            this.save();
                            cx.notify();
                        }))
                        .aria_label(title.clone())
                        .relative()
                        .flex()
                        .items_center()
                        .justify_center()
                        .px(px(if multi { m.gap } else { 0. }))
                        .py(px(3.))
                        .rounded(px(m.control_radius))
                        .when(multi && selected, |s| s.bg(rgb(c.floating)))
                        .when(ghost, |s| s.opacity(0.4))
                        .text_color(rgb(if selected { c.text } else { c.secondary }))
                        .text_size(px(t.typography.title))
                        .font_weight(FontWeight::MEDIUM)
                        .when(multi, |s| s.flex_auto().min_w(px(0.)).overflow_hidden())
                        .child(div().min_w(px(0.)).truncate().child(title.clone()));
                    if closable {
                        // Mac tabs: the close button sits at the leading edge, on hover.
                        cell = cell.child(
                            div()
                                .id(format!("close-{tab}"))
                                .aria_label(format!("Close {title}"))
                                .role(Role::Button)
                                .tab_index(0)
                                .focus_style(|s| {
                                    s.opacity(1.)
                                        .ring(px(m.border * 2.))
                                        .ring_color(rgb(c.focus))
                                })
                                .absolute()
                                .left(px(m.border))
                                .top_0()
                                .bottom_0()
                                .flex()
                                .items_center()
                                .opacity(0.)
                                .group_hover_style(tab_group.clone(), |s| s.opacity(1.))
                                .child(
                                    div()
                                        .rounded(px(m.inner_radius))
                                        .hover_style(|s| s.bg(rgb(c.hover)))
                                        .child(crate::symbol::symbol(
                                            "xmark",
                                            m.symbol_inline,
                                            crate::symbol::Weight::Medium,
                                            c.secondary,
                                            w,
                                        )),
                                )
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.shell_command(&format!("panel:close:{close_id}"), cx);
                                })),
                        );
                    }
                    strip = strip.child(
                        cell.on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.layout.docks[p.dock].groups[p.group].active = ti;
                                this.begin_panel_drag(&tabid, e.position, false);
                                window.focus(&this.focus, cx);
                                cx.notify();
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.tab_menu(&menu_id, e.position, cx);
                            }),
                        ),
                    );
                }
                if !tab_track.overflow.is_empty() {
                    let entries: Vec<_> = tab_track
                        .overflow
                        .iter()
                        .map(|&i| {
                            MenuEntry::item(
                                layout::panel_title(&tabs[i]),
                                format!("panel:activate:{}", tabs[i]),
                            )
                            .checked(i == active)
                        })
                        .collect();
                    strip = strip.child(
                        div()
                            .id(format!("more-panels-{}-{}", p.dock, p.group))
                            .role(Role::Button)
                            .aria_label("More Panels")
                            .tab_index(0)
                            .w(px(m.hit))
                            .child(crate::symbol::symbol(
                                "chevron.down",
                                m.symbol_inline,
                                crate::symbol::Weight::Medium,
                                c.secondary,
                                w,
                            ))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, e: &ClickEvent, _, _| {
                                if let Some(host) = &this.host {
                                    let at = e.position();
                                    host.borrow()
                                        .context_menu(&entries, at.x.into(), at.y.into());
                                }
                            })),
                    );
                }
                header = header.child(strip);
            }
            if self.maximized_panel.is_some() {
                header = header.child(
                    div()
                        .id("restore-panel")
                        .role(Role::Button)
                        .tab_index(0)
                        .aria_label("Restore Panel")
                        .child(crate::symbol::symbol(
                            "arrow.down.right.and.arrow.up.left",
                            m.symbol_inline,
                            crate::symbol::Weight::Medium,
                            c.secondary,
                            w,
                        ))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.maximized_panel = None;
                            cx.notify();
                        })),
                );
            }
            if let Some(custom) = custom {
                header = header.child(
                    div()
                        .when(multi, |d| d.flex_none())
                        .when(!multi, |d| d.flex_1().min_w(px(0.)))
                        .h_full()
                        .child(custom),
                );
            } else if !is_viewer {
                let (dock, gi) = (p.dock, p.group);
                let add_group: SharedString = format!("add-{}-{}", p.dock, p.group).into();
                header = header.child(
                    div()
                        .id(format!("add-{}-{}", p.dock, p.group))
                        .group(add_group.clone())
                        .role(Role::Button)
                        .aria_label("Add Panel")
                        .tab_index(tabs.len() as isize)
                        .focus_style(|s| s.ring(px(m.border * 2.)).ring_color(rgb(c.focus)))
                        .on_click(cx.listener(move |this, e: &ClickEvent, _, _| {
                            this.add_menu(dock, gi, e.position())
                        }))
                        .tooltip(tooltip(&t, "Add Panel"))
                        .ml_auto()
                        .relative()
                        .size(px(m.hit))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(m.control_radius))
                        .hover_style(|s| s.bg(rgb(c.hover)))
                        .child(crate::symbol::symbol(
                            "plus",
                            m.symbol_panel,
                            crate::symbol::Weight::Medium,
                            c.secondary,
                            w,
                        ))
                        // `text` on hover (language §8): a second glyph fades in.
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .opacity(0.)
                                .group_hover_style(add_group, |s| s.opacity(1.))
                                .child(crate::symbol::symbol(
                                    "plus",
                                    m.symbol_panel,
                                    crate::symbol::Weight::Medium,
                                    c.text,
                                    w,
                                )),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                );
            }
            panel = panel.child(header);
            let content = self
                .integration
                .as_ref()
                .and_then(|i| i.panel(&id, p.rect, w, cx));
            if let Some(content) = content {
                panel = panel.child(content);
            } else {
                let (heading, line, action) = layout::empty_state(&id);
                let mut empty = empty_state(&t, heading, line).h(px((p.rect.h - m.header).max(0.)));
                // Actions run only with an app attached (the bare shell has no importer).
                if let Some((label, command)) = action.filter(|_| self.integration.is_some()) {
                    empty = empty.child(
                        div()
                            .id(format!("empty-action-{id}"))
                            .role(Role::Button)
                            .aria_label(label)
                            .tab_index(100)
                            .focus_style(|s| s.ring(px(m.border * 2.)).ring_color(rgb(c.focus)))
                            .mt(px(m.gap))
                            .h(px(m.control))
                            .px(px(m.inset))
                            .flex()
                            .items_center()
                            .rounded(px(m.control_radius))
                            .bg(rgb(c.field))
                            .hover_style(|s| s.bg(rgb(c.hover)))
                            .text_size(px(t.typography.reading))
                            .text_color(rgb(c.text))
                            .child(label)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let cmd = Command {
                                    name: command.into(),
                                    args: vec![],
                                    token: None,
                                };
                                this.command(cmd, window, cx);
                            })),
                    );
                }
                panel = panel.child(empty);
            }
            root = root.child(panel);
        }
        if let Some(integration) = &self.integration {
            let shown: Vec<&str> = native_bodies.iter().map(|(id, _)| id.as_str()).collect();
            integration.shown_panels(&shown, cx);
        }
        if let Some(h) = &self.host {
            *h.borrow().native_panels.borrow_mut() = native_bodies;
        }
        for (i, d) in self.geometry.dividers.clone().into_iter().enumerate() {
            let divd = d.clone();
            let key_divider = d.clone();
            root = root.child(
                positioned(d.rect)
                    .id(format!("splitter-{i}"))
                    .role(Role::Splitter)
                    .aria_label("Resize dock")
                    .tab_index(1000 + i as isize)
                    .focus_style(|s| s.bg(rgb(c.focus)))
                    .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                        let delta = match e.keystroke.key.as_str() {
                            "left" | "up" => -this.tokens.metrics.keyboard_step,
                            "right" | "down" => this.tokens.metrics.keyboard_step,
                            "enter" if key_divider.group.is_none() => {
                                this.layout.docks[key_divider.dock].collapsed =
                                    !this.layout.docks[key_divider.dock].collapsed;
                                0.
                            }
                            _ => return,
                        };
                        this.layout.resize(&key_divider, delta, &this.tokens);
                        this.save();
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .cursor(if d.horizontal {
                        CursorStyle::ResizeUpDown
                    } else {
                        CursorStyle::ResizeLeftRight
                    })
                    .hover_style(|s| s.bg(rgb(c.separator)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            this.split_focus = Some(divd.clone());
                            if let Some(h) = &this.host {
                                h.borrow_mut().splitter_focus = true;
                            }
                            this.gesture = Some(Gesture::Resize(
                                divd.clone(),
                                e.position,
                                this.layout.clone(),
                            ));
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }),
                    ),
            );
        }
        // Drop preview and a ghost label that follows the pointer. Work-surface
        // overlays stay solid tints (no glass below the controls band).
        if let Some(Gesture::Panel(drag)) = &self.gesture
            && drag.moved
        {
            if let Some(p) = drag.preview {
                // Only the drop zone: `accent_wash` with a 2 pt accent outline.
                root = root.child(
                    positioned(p.rect)
                        .id("drop-preview")
                        .rounded(px(m.panel_radius))
                        .border(px(m.drop_stroke))
                        .border_color(rgb(c.accent))
                        .bg(rgb(c.accent_wash)),
                );
            }
            root = root.child(
                div()
                    .id("drag-ghost")
                    .absolute()
                    .left(drag.at.x + px(12.))
                    .top(drag.at.y + px(10.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(m.control_radius))
                    .bg(rgb(c.floating))
                    .border(px(m.border))
                    .border_color(rgb(c.separator))
                    .shadow_md()
                    .font_weight(FontWeight::MEDIUM)
                    .child(layout::panel_title(&drag.id).to_string()),
            );
        }
        let workspace = layout::PRESETS
            .iter()
            .find(|p| p.0 == self.layout.workspace)
            .map_or("Custom", |p| p.1);
        // Status bar (language §2): quiet facts. Left: document facts (or a
        // message); right: the workspace pop-up.
        let left = self
            .integration
            .as_ref()
            .map_or_else(|| self.status.clone(), |i| i.status(cx));
        let left = if left.is_empty() {
            self.status.clone()
        } else {
            left
        };
        root.child(
            positioned(Rect {
                x: 0.,
                y: f32::from(viewport.height) - m.status,
                w: f32::from(viewport.width),
                h: m.status,
            })
            .id("status-bar")
            .accessibility_id("shell.status")
            .role(Role::Status)
            .aria_label("Status")
            .aria_value(left.clone())
            .flex()
            .items_center()
            .gap(px(m.inset))
            .px(px(m.inset + m.gap))
            .text_size(px(t.typography.minimum))
            .text_color(rgb(c.muted))
            .child(div().flex_1().min_w(px(0.)).truncate().child(left))
            .child(
                div()
                    .id("status-workspace")
                    .track_focus(&self.workspace_focus)
                    .tab_index(2000)
                    .focus_style(|s| s.ring(px(m.border * 2.)).ring_color(rgb(c.focus)))
                    .on_click(
                        cx.listener(|this, e: &ClickEvent, _, _| this.workspace_menu(e.position())),
                    )
                    .role(Role::Button)
                    .aria_label(format!("Workspace: {workspace}"))
                    .flex()
                    .items_center()
                    .gap(px(m.border * 2.))
                    .px(px(m.gap))
                    .h(px(m.hit))
                    .rounded(px(m.control_radius))
                    .hover_style(|s| s.bg(rgb(c.hover)).text_color(rgb(c.text)))
                    .child(workspace.to_string())
                    .child(crate::symbol::symbol(
                        "chevron.up.chevron.down",
                        m.symbol_inline - 2.,
                        crate::symbol::Weight::Medium,
                        c.muted,
                        w,
                    )),
            ),
        )
    }
}

#[cfg(test)]
mod keyboard_tests {
    use super::*;
    use std::sync::Arc;

    fn app() -> HeadlessAppContext {
        HeadlessAppContext::with_platform(
            Arc::new(gpui_parley::ParleyTextSystem::new(
                gpui_parley::SystemFonts::Load,
            )),
            Arc::new(()),
            || None,
        )
    }
    fn command(name: &str) -> Command {
        Command {
            name: name.into(),
            args: vec![],
            token: None,
        }
    }

    #[::core::prelude::v1::test]
    fn panels_cycle_both_ways_and_skip_closed_panels() {
        let mut cx = app();
        let window = cx
            .open_window(size(px(1440.), px(900.)), |_, cx| {
                cx.new(|cx| {
                    Editor::new(
                        Tokens::new(pm_design::Scheme::Dark, Default::default()),
                        None,
                        cx,
                    )
                })
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, w, cx| {
                assert_eq!(
                    v.panel_order,
                    ["assets", "viewer", "inspector", "timeline", "fxbrowser"]
                );
                let order = v.panel_order.clone();
                for id in order.iter().chain(order.iter().take(1)) {
                    v.command(command("shellPanelNext"), w, cx);
                    assert!(v.panel_focus[id].is_focused(w), "{id}");
                    assert_eq!(
                        v.ringed_panel.as_deref(),
                        Some(id.as_str()),
                        "Ctrl-F6 rings it"
                    );
                }
                v.command(command("shellPanelPrevious"), w, cx);
                assert!(v.panel_focus[order.last().unwrap()].is_focused(w));
                w.focus(&v.panel_focus["timeline"], cx);
                v.run_shell_command("panel:close:fxbrowser", cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, w, cx| {
                assert!(!v.panel_order.iter().any(|id| id == "fxbrowser"));
                v.command(command("shellPanelPrevious"), w, cx);
                assert!(v.panel_focus["inspector"].is_focused(w));
            })
            .unwrap();
    }

    #[::core::prelude::v1::test]
    fn tab_and_shift_tab_are_inverse_and_include_groups_controls_and_status() {
        let mut cx = app();
        let window = cx
            .open_window(size(px(1440.), px(900.)), |_, cx| {
                cx.new(|cx| {
                    Editor::new(
                        Tokens::new(pm_design::Scheme::Dark, Default::default()),
                        None,
                        cx,
                    )
                })
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, w, cx| {
                let first = v.panel_focus[&v.panel_order[0]].clone();
                w.focus(&first, cx);
                let mut stops = vec![first.clone()];
                for _ in 0..100 {
                    v.command(command("shellFocusNext"), w, cx);
                    let focused = w.focused(cx).expect("Tab must have a focused element");
                    if focused == first {
                        break;
                    }
                    assert!(
                        !stops.contains(&focused),
                        "focus loop before returning to start"
                    );
                    stops.push(focused);
                }
                assert!(first.is_focused(w), "Tab wraps");
                assert_eq!(
                    stops.last(),
                    Some(&v.workspace_focus),
                    "workspace follows panels/splitters"
                );
                for handle in v.panel_order.iter().map(|id| &v.panel_focus[id]) {
                    assert!(stops.contains(handle));
                }
                assert!(
                    stops.len() > v.panel_order.len() * 2,
                    "tabs/add controls/splitters are reachable"
                );
                for expected in stops.iter().rev() {
                    v.command(command("shellFocusPrevious"), w, cx);
                    assert_eq!(w.focused(cx).as_ref(), Some(expected));
                }
            })
            .unwrap();
    }
    #[::core::prelude::v1::test]
    fn keyboard_close_does_not_activate_the_removed_parent_tab() {
        let mut cx = app();
        let window = cx
            .open_window(size(px(1440.), px(900.)), |_, cx| {
                cx.new(|cx| {
                    Editor::new(
                        Tokens::new(pm_design::Scheme::Dark, Default::default()),
                        None,
                        cx,
                    )
                })
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, w, cx| {
                w.focus(&v.panel_focus["assets"], cx);
                // Project tab, Close Project, Effect Controls tab, Close Effect Controls.
                for _ in 0..4 {
                    w.focus_next(cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, w, cx| {
            let keystroke = Keystroke::parse("enter").unwrap();
            w.dispatch_event(
                KeyDownEvent {
                    keystroke: keystroke.clone(),
                    is_held: false,
                    prefer_character_input: false,
                }
                .to_platform_input(),
                cx,
            );
            w.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
        })
        .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |v, _, _| {
                assert!(!v.layout.open_panels().contains(&"layer-effects"));
                let group = &v.layout.docks[0].groups[0];
                assert_eq!(group.tabs, ["assets"]);
                assert_eq!(group.active, 0);
            })
            .unwrap();
    }
}
