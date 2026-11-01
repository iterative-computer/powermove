use crate::{
    layout::{self, Divider, DropPreview, DropTarget, Edge, Geometry, Layout, LayoutStore, Rect},
    native::{Command, Host, MenuState},
    session::{Session, WindowState},
};
use gpui::{prelude::*, *};
use pm_design::Tokens;
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

/// A tab drag in progress: the layout it started from, the pointer and the
/// target it would drop on. Escape restores `original`.
#[derive(Clone)]
struct PanelDrag {
    id: String,
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
    /// Docking area of the last render, in window points.
    dock_bounds: Rect,
    gesture: Option<Gesture>,
    focus: FocusHandle,
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
            Ok(s) => (s.unwrap_or_default(), "Ready".into()),
            Err(e) => (
                LayoutStore::default(),
                format!("Layout could not be restored: {e}"),
            ),
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
            dock_bounds: Rect::default(),
            gesture: None,
            focus: cx.focus_handle(),
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
            self.tokens = Tokens::new(
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
    pub fn set_document_metadata(
        &mut self,
        title: &str,
        edited: bool,
        path: Option<&std::path::Path>,
    ) {
        if let Some(host) = &self.host {
            host.borrow().set_document_metadata(title, edited, path);
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
    fn menu_state(&self) -> MenuState {
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
            ("panel", "show") => self.layout.show_panel(rest),
            ("panel", "close") => self.layout.close_panel(rest),
            ("panel", "dock") => {
                let (id, edge) = rest.rsplit_once(':').unwrap_or((rest, ""));
                let edge = match edge {
                    "left" => Edge::Left,
                    "right" => Edge::Right,
                    _ => Edge::Bottom,
                };
                self.layout.move_panel_to(id, DropTarget::Edge(edge))
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
                    h.borrow().prompt_layout_name(&self.layout.workspace);
                }
                Ok(())
            }
            ("layout", "save") => {
                let layout = self.layout.clone();
                let mut saved = String::new();
                self.mutate_store(|s| s.save_named(rest, &layout).map(|n| saved = n))
                    .map(|_| self.status = format!("Saved layout “{saved}”"))
            }
            ("layout", "apply") => self
                .store
                .named(rest, &self.layout.workspace)
                .map(|l| self.layout = l)
                .ok_or_else(|| anyhow::anyhow!("No layout named “{rest}”")),
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
            Ok(()) => self.save(),
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
        eprintln!(
            "shell command={} args={:?} token={:?}",
            cmd.name, cmd.args, cmd.token
        );
        if cmd.name.starts_with("tool") && cmd.name.len() > 4 {
            // Keep the toolbar's tool group in step with keyboard/menu selection.
            self.tool = cmd.name.clone();
            if let Some(h) = &self.host {
                h.borrow().select_tool(&cmd.name);
            }
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
                    h.borrow_mut().open_palette(&self.menu_state());
                }
            }
            "fieldMenu" => {
                if let Some(h) = &self.host {
                    h.borrow_mut().popup(true);
                }
            }
            "toggleSidebar" | "toggleInspector" => {
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
            "shellAppearanceChanged" => {
                self.tokens = Tokens::new(self.tokens.scheme, crate::native::settings());
                if let Some(h) = &self.host {
                    h.borrow_mut().update_tokens(self.tokens.clone());
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
                self.save();
                self.layout = self.store.get(&self.project, &name[10..]);
                self.save();
            }
            name if name.starts_with("tool") && name.len() > 4 => {}
            name if self.shell_command(name, cx) => {}
            name => {
                handled = self.host.as_ref().is_some_and(|h| h.borrow().system(name));
                if !handled {
                    self.status = "Command is not available yet".into();
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
        let preview = self
            .layout
            .drop_target(&self.geometry, self.dock_bounds, x, y, &self.tokens);
        self.gesture = Some(Gesture::Panel(PanelDrag {
            id: id.into(),
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
        self.layout.move_panel_to(&drag.id, target).ok()?;
        self.save();
        Some(target)
    }
    /// Test-driver commands (`PM_PERF_GATE=1` runs only): drag a tab to a point,
    /// drop it, open a tab's context menu, close transients.
    fn test_command(&mut self, cmd: &Command, cx: &mut Context<Self>) {
        if std::env::var("PM_PERF_GATE").as_deref() != Ok("1") {
            return;
        }
        let s = |i: usize| cmd.args.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
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
            "tabMenu" => self.tab_menu(&s(0), point(px(f(1)), px(f(2)))),
            "dismiss" => {
                if let Some(h) = &self.host {
                    h.borrow().dismiss_transients();
                }
            }
            _ => {}
        }
        cx.notify();
    }
    /// Runs a shell command without an AppKit host (tests).
    #[doc(hidden)]
    pub fn run_shell_command(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        self.shell_command(name, cx)
    }
    fn tab_menu(&self, id: &str, position: Point<Pixels>) {
        let Some(h) = &self.host else { return };
        let title = layout::panel_title(id);
        let mut items = vec![];
        if id != "viewer" {
            items.push((format!("Close {title}"), format!("panel:close:{id}")));
        }
        items.push((String::new(), String::new()));
        for (label, edge) in [
            ("Dock Left", "left"),
            ("Dock Right", "right"),
            ("Dock Bottom", "bottom"),
        ] {
            items.push((label.to_string(), format!("panel:dock:{id}:{edge}")));
        }
        items.push((String::new(), String::new()));
        items.push(("Save Layout…".into(), "layout:save".into()));
        items.push(("Reset Workspace".into(), "layout:reset".into()));
        h.borrow()
            .context_menu(&items, f32::from(position.x), f32::from(position.y));
    }
    fn add_menu(&self, dock: usize, group: usize, position: Point<Pixels>) {
        let Some(h) = &self.host else { return };
        let open = self.layout.open_panels();
        let items: Vec<_> = layout::PANELS
            .iter()
            .filter(|p| !open.contains(p))
            .map(|p| {
                (
                    layout::panel_title(p).to_string(),
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
fn positioned(r: Rect) -> Div {
    div()
        .absolute()
        .left(px(r.x))
        .top(px(r.y))
        .w(px(r.w))
        .h(px(r.h))
}
/// `rgb` with an alpha channel (0–255) for overlays drawn from opaque tokens.
fn tint(rgb: u32, alpha: u8) -> Rgba {
    gpui::rgba((rgb << 8) | alpha as u32)
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
        self.geometry = self.layout.geometry(self.dock_bounds, &t);
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
            h.publish_menu_state(&self.menu_state());
        }
        let native = self.host.is_some();
        let canvas_viewer = viewer;
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
                        let mut rects = vec![];
                        if let Some(v) = canvas_viewer.filter(|_| native) {
                            rects.extend([
                                Rect {
                                    x: 0.,
                                    y: 0.,
                                    w: width,
                                    h: v.y,
                                },
                                Rect {
                                    x: 0.,
                                    y: v.y,
                                    w: v.x,
                                    h: v.h,
                                },
                                Rect {
                                    x: v.x + v.w,
                                    y: v.y,
                                    w: width - v.x - v.w,
                                    h: v.h,
                                },
                                Rect {
                                    x: 0.,
                                    y: v.y + v.h,
                                    w: width,
                                    h: height - v.y - v.h,
                                },
                            ]);
                        } else {
                            rects.push(Rect {
                                x: 0.,
                                y: 0.,
                                w: width,
                                h: height,
                            });
                        }
                        for r in rects {
                            window.paint_quad(
                                PaintQuad::new(Bounds::new(
                                    point(px(r.x), px(r.y)),
                                    gpui::size(px(r.w), px(r.h)),
                                ))
                                .background(rgb(c.window)),
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
                                    this.layout.drop_target(
                                        &this.geometry,
                                        this.dock_bounds,
                                        f32::from(e.position.x),
                                        f32::from(e.position.y),
                                        &this.tokens,
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
                            && let Err(err) = this.layout.move_panel_to(&drag.id, p.target)
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
                .child("Untitled · Edited")
                .child(div().ml_auto().child("Tools   ·   Agent   ·   Export")),
            );
        }
        let dragged = match &self.gesture {
            Some(Gesture::Panel(d)) if d.moved => Some(d.id.clone()),
            _ => None,
        };
        for p in self.geometry.panels.clone() {
            let group = &self.layout.docks[p.dock].groups[p.group];
            let id = group.active().to_string();
            let is_viewer = id == "viewer";
            let tabs = group.tabs.clone();
            let active = group.active;
            let header_bg = if is_viewer { c.viewer } else { c.panel };
            let mut panel = positioned(p.rect)
                .id(format!("group-{}-{}", p.dock, p.group))
                .role(Role::Group)
                .accessibility_id(format!("shell.panel.{id}"))
                .aria_label(layout::panel_title(&id).to_string())
                .rounded(px(m.panel_radius))
                .border(px(m.border))
                .border_color(rgb(c.separator))
                .overflow_hidden();
            if !is_viewer || !native {
                panel = panel.bg(rgb(if is_viewer { c.viewer } else { c.panel }));
            }
            let group_name: SharedString = format!("tabs-{}-{}", p.dock, p.group).into();
            let mut header = div()
                .group(group_name.clone())
                .flex()
                .items_center()
                .gap(px(m.gap))
                .h(px(m.header))
                .px(px(m.inset))
                .bg(rgb(header_bg));
            // Approved mockup: a sunken segmented strip for tab groups, a bold
            // title for single panels. The whole tab is the drag handle.
            let multi = tabs.len() > 1;
            let mut strip = div().flex().items_center().gap(px(2.)).when(multi, |s| {
                s.bg(rgb(c.window))
                    .rounded(px(m.control_radius + 2.))
                    .p(px(2.))
            });
            for (ti, tab) in tabs.iter().enumerate() {
                let tabid = tab.clone();
                let menu_id = tab.clone();
                let selected = ti == active;
                let ghost = dragged.as_deref() == Some(tab.as_str());
                let close_id = tab.clone();
                let closable = tab != "viewer";
                strip = strip.child(
                    div()
                        .id(format!("tab-{tab}"))
                        .role(Role::Tab)
                        .aria_label(layout::panel_title(tab).to_string())
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .px(px(if multi { 10. } else { 2. }))
                        .py(px(3.))
                        .rounded(px(m.control_radius))
                        .when(multi && selected, |s| s.bg(rgb(c.floating)))
                        .when(ghost, |s| s.opacity(0.4))
                        .text_color(rgb(if selected { c.text } else { c.secondary }))
                        .when(!multi, |s| s.font_weight(FontWeight::SEMIBOLD))
                        .when(multi, |s| s.font_weight(FontWeight::MEDIUM))
                        .cursor_pointer()
                        .child(layout::panel_title(tab).to_string())
                        .when(closable && selected, |s| {
                            s.child(
                                div()
                                    .id(format!("close-{tab}"))
                                    .aria_label(format!("Close {}", layout::panel_title(tab)))
                                    .role(Role::Button)
                                    .w(px(14.))
                                    .h(px(14.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded(px(7.))
                                    .text_size(px(t.typography.minimum))
                                    .text_color(rgb(c.muted))
                                    .opacity(0.)
                                    .group_hover_style(group_name.clone(), |s| s.opacity(1.))
                                    .hover_style(|s| s.bg(rgb(c.hover)))
                                    .child("×")
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.shell_command(&format!("panel:close:{close_id}"), cx);
                                    })),
                            )
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                this.layout.docks[p.dock].groups[p.group].active = ti;
                                this.gesture = Some(Gesture::Panel(PanelDrag {
                                    id: tabid.clone(),
                                    start: e.position,
                                    original: this.layout.clone(),
                                    at: e.position,
                                    moved: false,
                                    preview: None,
                                }));
                                this.split_focus = None;
                                if let Some(h) = &this.host {
                                    h.borrow_mut().splitter_focus = false;
                                }
                                window.focus(&this.focus, cx);
                                cx.notify();
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, _, _| {
                                this.tab_menu(&menu_id, e.position);
                            }),
                        ),
                );
            }
            header = header.child(strip);
            if !is_viewer {
                let (dock, gi) = (p.dock, p.group);
                header = header.child(
                    div()
                        .id(format!("add-{}-{}", p.dock, p.group))
                        .role(Role::Button)
                        .aria_label("Add Panel")
                        .ml_auto()
                        .w(px(20.))
                        .h(px(20.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(m.control_radius))
                        .text_color(rgb(c.muted))
                        .hover_style(|s| s.bg(rgb(c.hover)).text_color(rgb(c.text)))
                        .cursor_pointer()
                        .child("+")
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                this.add_menu(dock, gi, e.position);
                            }),
                        ),
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
                panel = panel.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px((p.rect.h - m.header).max(0.)))
                        .text_color(rgb(c.muted))
                        .child(if is_viewer {
                            "viewer".into()
                        } else {
                            id.clone()
                        }),
                );
            }
            root = root.child(panel);
        }
        for (i, d) in self.geometry.dividers.clone().into_iter().enumerate() {
            let divd = d.clone();
            root = root.child(
                positioned(d.rect)
                    .id(format!("splitter-{i}"))
                    .role(Role::Splitter)
                    .aria_label("Resize dock")
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
                root = root.child(
                    positioned(p.rect)
                        .id("drop-preview")
                        .rounded(px(m.panel_radius))
                        .border(px(2.))
                        .border_color(rgb(c.accent))
                        .bg(tint(c.accent, 0x2e)),
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
        root.child(
            positioned(Rect {
                x: 0.,
                y: f32::from(viewport.height) - m.status,
                w: f32::from(viewport.width),
                h: m.status,
            })
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(m.inset + 4.))
            .text_size(px(t.typography.minimum))
            .text_color(rgb(c.muted))
            .child(
                self.integration
                    .as_ref()
                    .map_or_else(|| self.status.clone(), |i| i.status(cx)),
            )
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().w(px(7.)).h(px(7.)).rounded(px(4.)).bg(rgb(0x34c77b)))
                    .child("Metal"),
            )
            .child(workspace.to_string())
            .child(if self.integration.is_none() {
                "Paused"
            } else {
                ""
            }),
        )
    }
}
