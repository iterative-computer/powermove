use crate::kit::Recipes as _;
use crate::{
    layout::{self, Divider, DropPreview, DropTarget, Edge, Geometry, Layout, Rect},
    native::{Command, Host, MenuEntry, MenuState},
    session::WindowState,
    workspace,
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
    /// Panels sliding to a new place after a drop, a maximise or a
    /// workspace switch: each panel's offset from its new rect, springing to
    /// nothing (v1 moved panels the same way). Live resizes never slide.
    slides: PanelSlides,
    maximized_panel: Option<String>,
    /// Tabs of panels whose owner went away for a moment (an extension
    /// reloading), and where they were: the panel returns there.
    /// Parked panels: where each goes back, and the tab that was showing in
    /// its stack if it wasn't that panel, which shows again when it returns.
    parked: HashMap<String, (DropTarget, Option<String>)>,
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
    /// This window's copy of the kernel's workspace store.
    workspaces: Rc<workspace::Store>,
    /// The active workspace record as this window last read or wrote it.
    record: serde_json::Value,
    /// Records this window wrote that the store has not shown back yet.
    pending: Vec<serde_json::Value>,
    /// What the store last said this window should show.
    seen: Option<serde_json::Value>,
    /// The workspace this window last showed by choice (restored or
    /// activated); a new window follows the store's last activated one.
    preferred: Option<String>,
    /// Writes for the kernel, sent on the next frame.
    outbox: Vec<workspace::Write>,
    /// The status last passed to the app's status line.
    noted: String,
    project: String,
    split_focus: Option<Divider>,
    /// Session window id; 0 until the window is registered (tests, smoke).
    pub window_id: u64,
    session_path: Option<PathBuf>,
    session_pending: bool,
    tool: String,
    /// A passing message in the status bar (the original app's toasts:
    /// “Moved Effects above Viewer”, “Move cancelled”), and when it came.
    notice: Option<(String, std::time::Instant)>,
    /// The splitter the pointer is over (dock, group, horizontal) and how far
    /// along it, for the glow that follows the pointer.
    split_hover: Option<((usize, Option<usize>, bool), f32)>,
    /// A drag of a splitter between two panels has passed its dead zone.
    resize_live: bool,
}
/// The original app's name for a dock: “Left dock”, “Bottom dock”.
fn dock_label(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => format!("{}{} dock", first.to_uppercase(), chars.as_str()),
        None => "Dock".into(),
    }
}
/// The window session file. `PM_NATIVE_SHELL_STATE` names an isolated state
/// file; its directory then holds the session.
pub fn session_path() -> Option<PathBuf> {
    let state = std::env::var_os("PM_NATIVE_SHELL_STATE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                PathBuf::from(h)
                    .join("Library/Application Support/Powermove Native/Shell/layouts.json")
            })
        })?;
    Some(state.with_file_name("session.json"))
}
impl Editor {
    pub fn new(tokens: Tokens, host: Option<Rc<RefCell<Host>>>, cx: &mut Context<Self>) -> Self {
        let session_path = if host.is_none() { None } else { session_path() };
        let workspaces = Rc::new(workspace::Store::local());
        let record = workspaces.active(None);
        let layout = workspace::layout(&record);
        let project = "shell-untitled".to_string();
        Self {
            integration: None,
            layout,
            tokens,
            host,
            geometry: Geometry::default(),
            slides: PanelSlides::default(),
            maximized_panel: None,
            parked: HashMap::new(),
            dock_bounds: Rect::default(),
            gesture: None,
            focus: cx.focus_handle(),
            panel_focus: HashMap::new(),
            ringed_panel: None,
            panel_order: vec![],
            workspace_focus: cx.focus_handle().tab_stop(true).tab_index(2000),
            status: String::new(),
            workspaces,
            record,
            pending: vec![],
            seen: None,
            preferred: None,
            outbox: vec![],
            noted: String::new(),
            project,
            split_focus: None,
            window_id: 0,
            session_path,
            session_pending: false,
            tool: "toolSelect".into(),
            notice: None,
            split_hover: None,
            resize_live: false,
        }
    }
    /// Shows `text` in the status bar for a moment.
    fn notify_status(&mut self, text: String, cx: &mut Context<Self>) {
        let at = std::time::Instant::now();
        // As long as the app's other status notes hold.
        let hold = Duration::from_secs_f64(self.tokens.motion.note_hold);
        self.notice = Some((text, at));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(hold).await;
            let _ = this.update(cx, |v, cx| {
                if v.notice.as_ref().is_some_and(|(_, when)| *when == at) {
                    v.notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
    /// The dragged panel's name, or its stack's names joined (“Effects +
    /// Takes”), as the original app's drag card says it.
    fn moving_name(&self, id: &str, stack: bool, panels: &layout::Panels) -> String {
        match self.layout.find(id) {
            Some((d, g, _)) if stack => self.layout.docks[d].groups[g]
                .tabs
                .iter()
                .map(|t| panels.title(t).to_string())
                .collect::<Vec<_>>()
                .join(" + "),
            _ => panels.title(id).to_string(),
        }
    }
    /// The panel or dock a drop is named after, in this layout.
    fn near_name(&self, near: layout::Near, panels: &layout::Panels) -> (String, &'static str) {
        let title = |d: usize, g: usize| {
            self.layout
                .docks
                .get(d)
                .and_then(|dock| dock.groups.get(g))
                .map_or_else(String::new, |g| panels.title(g.active()).to_string())
        };
        match near {
            layout::Near::Tab(d, g) => (title(d, g), "tab"),
            layout::Near::Above(d, g) => (title(d, g), "above"),
            layout::Near::Below(d, g) => (title(d, g), "below"),
            layout::Near::Dock(d) => (
                dock_label(self.layout.docks.get(d).map_or("", |d| d.id.as_str())),
                "dock",
            ),
            layout::Near::Edge(e) => (dock_label(e.dock_id()), "dock"),
            layout::Near::Nothing => (String::new(), "dock"),
        }
    }
    /// The drag card's second line: where the panel would go.
    fn destination(&self, drag: &PanelDrag, panels: &layout::Panels) -> String {
        if let Some(p) = drag.preview {
            let (name, kind) = self.near_name(p.near, panels);
            return match kind {
                "tab" => format!("Tab with {name}"),
                "above" => format!("Above {name}"),
                "below" => format!("Below {name}"),
                _ => format!("Place in {name}"),
            };
        }
        match self.layout.drop_outcome(
            &self.geometry,
            self.dock_bounds,
            (f32::from(drag.at.x), f32::from(drag.at.y)),
            &self.tokens,
            &drag.id,
            drag.stack,
        ) {
            layout::DropOutcome::Here => "Already here".into(),
            _ => "Not a drop zone".into(),
        }
    }
    /// A drag in progress put back as it began: Escape, or the window
    /// losing focus (the original app cancels then).
    pub fn cancel_gesture(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.gesture.take() else {
            return;
        };
        let moved = matches!(&g, Gesture::Panel(d) if d.moved);
        self.layout = g.original();
        self.resize_live = false;
        if moved {
            self.notify_status("Move cancelled".into(), cx);
        }
        cx.notify();
    }
    /// Binds this editor to a session window and restores its project/workspace.
    pub fn restore(&mut self, state: &WindowState) {
        self.window_id = state.id;
        self.project = state.project.clone();
        self.preferred = Some(state.workspace.clone());
        self.show_record(self.workspaces.active(self.preferred.as_deref()));
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
        self.apply_themes(patch.clone(), patch, cx);
    }
    /// A theme with its own dark and light patches (`theme::set_schemes`).
    pub fn apply_themes(
        &mut self,
        dark: crate::theme::Patch,
        light: crate::theme::Patch,
        cx: &mut Context<Self>,
    ) {
        match crate::theme::set_schemes(dark, light) {
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
        self.project = project;
        // The project's own workspace arrives with the store; until then the
        // window keeps showing what it showed.
        self.seen = None;
        self.persist_session();
    }
    /// Writes the window's layout into its workspace record (the project's
    /// saved workspace and its entry in `workspaces`), then the session.
    pub fn save(&mut self) {
        // A parked panel keeps its place in the workspace (v1 kept a retired
        // panel's spec); only this window's view leaves it out.
        let mut written = self.layout.clone();
        for (id, (place, showing)) in &self.parked {
            written.insert_at(id, *place);
            // It was the stack's showing tab: the saved stack still shows it,
            // not the tab the window fell back to while it was away.
            if showing.is_none()
                && let Some((dock, group, tab)) = written.find(id)
            {
                written.docks[dock].groups[group].active = tab;
            }
        }
        let after = workspace::write(&written, &self.record);
        if after != self.record {
            self.commit(after);
        }
        self.persist_session();
    }
    /// The window now shows `after`: the store learns it on the next frame.
    fn commit(&mut self, after: serde_json::Value) {
        let before = self
            .pending
            .last()
            .cloned()
            .or_else(|| self.workspaces.project.clone())
            .unwrap_or_default();
        let write = workspace::Write::Active {
            before,
            after: after.clone(),
        };
        Rc::make_mut(&mut self.workspaces).apply(&write);
        self.outbox.push(write);
        self.pending.push(after.clone());
        self.record = after;
    }
    fn show_record(&mut self, record: serde_json::Value) {
        self.maximized_panel = None;
        self.layout = workspace::layout(&record);
        self.record = record;
    }
    /// Shows workspace `record` in this window and makes it the active one.
    fn activate_record(&mut self, record: serde_json::Value) {
        self.preferred = workspace::record_id(&record).map(str::to_owned);
        self.show_record(record.clone());
        self.commit(record);
        self.persist_session();
    }
    /// Sends queued writes and takes in what the kernel's store says now. A
    /// record this window wrote changes nothing when it comes back; anyone
    /// else's change to the window's workspace (an extension's
    /// `workspace.mutate`, another window) relays the panels.
    fn sync_workspaces(&mut self, cx: &mut App) {
        if let Some(error) = crate::session::take_error() {
            self.status = error;
        }
        let Some(integration) = self.integration.clone() else {
            return;
        };
        if self.status != self.noted {
            self.noted = self.status.clone();
            if !self.status.is_empty() {
                integration.note(&self.status, cx);
            }
        }
        for write in std::mem::take(&mut self.outbox) {
            integration.write_workspace(write, cx);
        }
        let Some(store) = integration.workspaces(cx) else {
            return;
        };
        if !Rc::ptr_eq(&store, &self.workspaces) {
            self.workspaces = store;
        }
        let target = self.workspaces.active(self.preferred.as_deref());
        if self.seen.as_ref() == Some(&target) {
            return;
        }
        self.seen = Some(target.clone());
        if let Some(i) = self.pending.iter().position(|p| *p == target) {
            self.pending.drain(..=i);
            return;
        }
        if target != self.record {
            self.pending.clear();
            if self
                .workspaces
                .project
                .as_ref()
                .is_some_and(|p| self.workspaces.deleted(p))
            {
                // Another window deleted this project's workspace: it moves
                // to the fallback, and the project keeps that one.
                self.activate_record(target);
                return;
            }
            self.show_record(target);
            self.persist_session();
        }
    }
    /// Applies a Workspace Definition (S100): `args` are the text and the
    /// definition the sheet opened with. A workspace changed meanwhile is
    /// left alone; a built-in is never overwritten, it is saved as a copy.
    fn define_workspace(&mut self, args: &[serde_json::Value]) {
        let text = args.first().and_then(|v| v.as_str()).unwrap_or_default();
        let opened = args.get(1).and_then(|v| v.as_str()).unwrap_or_default();
        if serde_json::from_str::<serde_json::Value>(opened)
            .ok()
            .as_ref()
            != Some(&self.record)
        {
            self.status = "This workspace changed while you were editing. Edit it again to see the latest version.".into();
            return;
        }
        let mut value = match workspace::check_definition(text) {
            Ok(v) => v,
            Err(why) => {
                self.status = why;
                return;
            }
        };
        if workspace::is_builtin(&self.record) {
            let name = workspace::record_name(&self.record);
            if workspace::record_name(&value) == name {
                value["name"] = format!("{name} Copy").into();
            }
            value["id"] = workspace::new_id().into();
            value["builtin"] = false.into();
        } else {
            value["id"] = self.layout.workspace.clone().into();
        }
        self.activate_record(value);
        self.status = "Workspace updated".into();
    }
    /// The workspace records this window offers: (id, name, built-in).
    pub fn workspace_choices(&self) -> Vec<(String, String, bool)> {
        self.workspaces
            .offered()
            .into_iter()
            .map(|r| {
                (
                    workspace::record_id(r).unwrap_or_default().to_owned(),
                    workspace::record_name(r).to_owned(),
                    workspace::is_builtin(r),
                )
            })
            .collect()
    }
    /// The active workspace record.
    pub fn workspace_record(&self) -> &serde_json::Value {
        &self.record
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
    /// Queues this window's state for the shared session file (other windows
    /// write too): written off the main thread, in order (`session::write`).
    pub fn persist_session(&mut self) {
        if self.window_id == 0 {
            return;
        }
        let (Some(path), Some(state)) = (self.session_path.clone(), self.window_state()) else {
            return;
        };
        let key = self.host.as_ref().is_some_and(|h| h.borrow().is_key());
        crate::session::write(&path, crate::session::Change::Upsert { state, key });
    }
    /// Removes this window from the session (the user closed it).
    pub fn forget_session(&mut self) {
        if let Some(path) = self.session_path.clone()
            && self.window_id != 0
        {
            crate::session::write(&path, crate::session::Change::Remove(self.window_id));
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
    /// The panels this window offers.
    /// The panels this window offers: built-ins, extension panels, then the
    /// workspace's generated panels (its `custom` sections).
    pub fn panels(&self, cx: &App) -> layout::Panels {
        let mut contributed = self
            .integration
            .as_ref()
            .map(|i| i.contributed_panels(cx))
            .unwrap_or_default();
        for (id, title) in workspace::generated(&self.record) {
            if !contributed.iter().any(|(c, _)| *c == id) {
                contributed.push((id, title));
            }
        }
        layout::Panels::new(contributed)
    }
    fn menu_state(&self, cx: &App) -> MenuState {
        let panels = self.panels(cx);
        MenuState {
            palette_effects: self
                .integration
                .as_ref()
                .map(|i| i.palette_effects(cx))
                .unwrap_or_default(),
            palette_layers: self
                .integration
                .as_ref()
                .map(|i| i.palette_layers(cx))
                .unwrap_or_default(),
            panels: panels
                .ids()
                .into_iter()
                .map(|id| (id.to_owned(), panels.title(id).to_owned()))
                .collect(),
            open: self
                .layout
                .open_panels()
                .into_iter()
                .map(String::from)
                .collect(),
            workspaces: self.workspace_choices(),
            workspace: self.layout.workspace.clone(),
            sidebar: self.dock_visible("left"),
            inspector: self.dock_visible("right"),
            tool: self.tool.clone(),
            disabled: self
                .integration
                .as_ref()
                .map(|i| i.disabled(cx))
                .unwrap_or_default(),
            checked: self
                .integration
                .as_ref()
                .map(|i| i.checked(cx))
                .unwrap_or_default(),
            recent: self
                .integration
                .as_ref()
                .map(|i| i.recent(cx))
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
        // A command that rearranges the docks moves panels to their new
        // places (one that does not leaves every rect where it was).
        self.slides.armed = true;
        let mut parts = name.splitn(3, ':');
        let (Some(kind), Some(verb)) = (parts.next(), parts.next()) else {
            return false;
        };
        let rest = parts.next().unwrap_or("");
        let panels = self.panels(cx);
        let result = match (kind, verb) {
            ("panel", "toggle") => {
                if self.layout.open_panels().contains(&rest) && layout::closable(rest) {
                    self.layout.close_panel(rest)
                } else {
                    self.layout.show_panel(rest, &panels)
                }
            }
            ("panel", "activate") => {
                // Switching a tab preserves the maximized stack.
                let result = self.layout.show_panel(rest, &panels);
                if let Err(error) = result {
                    self.status = error.to_string();
                } else {
                    self.save();
                }
                cx.notify();
                return true;
            }
            ("panel", "show") => match self.parked.remove(rest) {
                // Coming back (an extension's panels re-register at launch) puts
                // it in its place without taking the stack's showing tab.
                Some((place, showing)) if self.layout.find(rest).is_none() => {
                    let mut next = self.layout.clone();
                    next.show_panel(rest, &panels)
                        .and_then(|_| next.move_panel_to(rest, place))
                        .map(|_| {
                            if let Some((dock, group, tab)) = showing.and_then(|id| next.find(&id))
                                && next
                                    .find(rest)
                                    .is_some_and(|(d, g, _)| (d, g) == (dock, group))
                            {
                                next.docks[dock].groups[group].active = tab;
                            }
                            self.layout = next
                        })
                        .or_else(|_| self.layout.show_panel(rest, &panels))
                }
                _ => self.layout.show_panel(rest, &panels),
            },
            ("panel", "close") => {
                // The frame that shows it closed: its feel line (`PM_FEEL_TRACE`).
                gpui_macos::feel::note_now("panel.close");
                self.parked.remove(rest);
                self.layout.close_panel(rest)
            }
            // panel:park:<id> — closed, but a later show puts it back in place.
            ("panel", "park") => match self.layout.find(rest) {
                Some((dock, group, index)) => {
                    // A tab alone in its stack takes the stack with it; it
                    // comes back as a stack in the same position.
                    let stack = &self.layout.docks[dock].groups[group];
                    let alone = stack.tabs.len() == 1;
                    let showing = Some(stack.tabs[stack.active].clone()).filter(|t| t != rest);
                    let place = if alone {
                        DropTarget::Split { dock, group }
                    } else {
                        DropTarget::Tab { dock, group, index }
                    };
                    self.parked.insert(rest.to_owned(), (place, showing));
                    self.layout.close_panel(rest)
                }
                None => Ok(()),
            },
            ("panel", "dock") => {
                let (id, edge) = rest.rsplit_once(':').unwrap_or((rest, ""));
                let edge = match edge {
                    "left" => Edge::Left,
                    "right" => Edge::Right,
                    "top" => Edge::Top,
                    _ => Edge::Bottom,
                };
                self.layout.move_panel_to(id, DropTarget::Edge(edge))
            }
            ("panel", "group" | "above" | "below" | "left" | "right") => {
                let (id, target) = rest.split_once(':').unwrap_or((rest, ""));
                self.layout.place_relative(id, target, verb)
            }
            // Maximize Panel, or Restore Panel Size on the maximized stack.
            ("panel", "maximize") => {
                let in_stack = |m: &str| {
                    self.layout.find(rest).is_some_and(|(d, g, _)| {
                        self.layout.docks[d].groups[g].tabs.iter().any(|t| t == m)
                    })
                };
                if self.maximized_panel.as_deref().is_some_and(in_stack) {
                    self.maximized_panel = None;
                } else if self.layout.open_panels().contains(&rest) {
                    self.maximized_panel = Some(rest.into());
                }
                cx.notify();
                return true;
            }
            // panel:tabwith:<target>:<id> — Add Panel as Tab: a closed panel
            // opens there, an open one moves there; it shows.
            ("panel", "tabwith") => {
                let (target, id) = rest.split_once(':').unwrap_or((rest, ""));
                let mut next = self.layout.clone();
                let opened = if next.find(id).is_none() {
                    next.show_panel(id, &panels)
                } else {
                    Ok(())
                };
                opened
                    .and_then(|_| next.place_relative(id, target, "group"))
                    .map(|_| self.layout = next)
            }
            ("panel", "closeothers") => {
                if let Some((d, g, _)) = self.layout.find(rest) {
                    for t in self.layout.docks[d].groups[g].tabs.clone() {
                        if t != rest {
                            self.parked.remove(&t);
                        }
                    }
                }
                self.layout.close_others(rest)
            }
            // panel:column:<id>:left|right — a new column beside its dock.
            ("panel", "column") => {
                let (id, side) = rest.rsplit_once(':').unwrap_or((rest, ""));
                match self.layout.find(id) {
                    Some((dock, ..)) => self.layout.move_panel_to(
                        id,
                        DropTarget::Beside {
                            dock,
                            after: side == "right",
                        },
                    ),
                    None => Err(anyhow::anyhow!("Unknown panel")),
                }
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
                        next.show_panel(id, &panels)
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
                    h.borrow()
                        .prompt_workspace_name(workspace::record_name(&self.record));
                }
                Ok(())
            }
            ("layout", "save") => layout::layout_name(rest).map(|name| {
                let mut copy = workspace::write(&self.layout, &self.record);
                copy["id"] = workspace::new_id().into();
                copy["name"] = name.clone().into();
                copy["builtin"] = false.into();
                copy["scope"] = "global".into();
                copy["projectId"] = serde_json::Value::Null;
                self.activate_record(copy);
                self.status = format!("Saved workspace “{name}”");
            }),
            ("layout", "edit") => {
                if let Some(h) = &self.host {
                    let json = serde_json::to_string_pretty(&self.record).unwrap_or_default();
                    h.borrow().edit_workspace_definition(&json);
                }
                Ok(())
            }
            ("layout", "delete") => match self.workspaces.get(rest).cloned() {
                Some(r) if workspace::is_builtin(&r) => {
                    Err(anyhow::anyhow!("Built-in workspaces can’t be deleted."))
                }
                Some(r) => {
                    let write = workspace::Write::Archive(rest.to_owned());
                    Rc::make_mut(&mut self.workspaces).apply(&write);
                    self.outbox.push(write);
                    if self.layout.workspace == rest {
                        let fallback = self.workspaces.fallback();
                        self.activate_record(fallback);
                    }
                    self.status = format!("Deleted workspace “{}”", workspace::record_name(&r));
                    Ok(())
                }
                None => Err(anyhow::anyhow!("That workspace no longer exists.")),
            },
            ("layout", "reset") => {
                // A built-in returns to how it shipped; a saved workspace keeps
                // its generated panels and settings and takes its base's panels.
                let id = self.layout.workspace.clone();
                let after = workspace::preset(&id).unwrap_or_else(|| {
                    let base = self.record["base"].as_str().unwrap_or(workspace::FALLBACK);
                    let mut l = Layout::preset(base);
                    l.workspace = id;
                    workspace::write(&l, &self.record)
                });
                self.activate_record(after);
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
    pub fn command(&mut self, cmd: Command, window: &mut Window, cx: &mut Context<Self>) {
        // Resolve a review drag from the actual divider, never screen constants.
        #[cfg(debug_assertions)]
        if let Some(rest) = cmd.name.strip_prefix("reviewSplitter:") {
            let parts: Vec<_> = rest.split(':').collect();
            if let [panel, action, offset] = parts.as_slice()
                && let Ok(offset) = offset.parse::<f32>()
                && let Some(placed) = self
                    .geometry
                    .panels
                    .iter()
                    .find(|p| self.layout.docks[p.dock].groups[p.group].active() == *panel)
                && let Some(divider) = self
                    .geometry
                    .dividers
                    .iter()
                    .find(|d| d.dock == placed.dock && d.group == Some(placed.group))
            {
                let (x, y) = (
                    divider.rect.x + divider.rect.w / 2.,
                    divider.rect.y + divider.rect.h / 2.,
                );
                let action = action.to_string();
                window.defer(cx, move |window, cx| {
                    crate::macos::review_splitter(&action, x, y, offset, window, cx);
                });
            }
            return;
        }
        self.slides.armed = true;
        // Development builds: `--open reviewViews` with `--save-dir` writes
        // the window's AppKit view tree there.
        #[cfg(debug_assertions)]
        if let Some((action, index)) = cmd
            .name
            .strip_prefix("reviewTab:")
            .and_then(|rest| rest.split_once(':'))
        {
            if let (Some(host), Ok(index)) = (&self.host, index.parse()) {
                host.borrow().review_tab(action, index);
            }
            return;
        }
        // `pointer:x:y`, `click:x:y`, `dblclick:x:y`, `drag:x:y:x:y`: pointer
        // input without the pointer, for a review of the running app. It is
        // dispatched after this update: input re-enters the window.
        #[cfg(debug_assertions)]
        if matches!(
            cmd.name.split(':').next(),
            Some("pointer" | "click" | "dblclick" | "drag" | "press" | "move" | "release")
        ) {
            let name = cmd.name.clone();
            window.defer(cx, move |window, cx| {
                crate::macos::review_pointer(&name, window, cx);
            });
            return;
        }
        // `reviewResize:-240:-140`: a stepped resize that measures whether
        // hosted controls stay attached (`resize.txt` in `--save-dir`).
        #[cfg(debug_assertions)]
        if let Some((dw, dh)) = cmd
            .name
            .strip_prefix("reviewResize:")
            .and_then(|rest| rest.split_once(':'))
        {
            if let (Some(host), Ok(dw), Ok(dh)) = (&self.host, dw.parse(), dh.parse()) {
                host.borrow().review_resize(dw, dh);
            }
            return;
        }
        // `reviewPopup:hover:3`, `leave:`, `type:exp`, `key:down`: the open
        // palette or menu, as the pointer and the keyboard drive it.
        #[cfg(debug_assertions)]
        if let Some((action, value)) = cmd
            .name
            .strip_prefix("reviewPopup:")
            .and_then(|rest| rest.split_once(':'))
        {
            if let Some(host) = &self.host {
                host.borrow().review_popup(action, value);
            }
            return;
        }
        #[cfg(debug_assertions)]
        if cmd.name == "reviewViews" {
            if let (Some(host), Ok(dir)) = (&self.host, std::env::var("PM_NATIVE_REVIEW_SAVE_DIR"))
            {
                let _ = std::fs::write(
                    std::path::Path::new(&dir).join("views.txt"),
                    host.borrow().view_tree(),
                );
            }
            return;
        }
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
            // The shell took the key before the window saw it: still a
            // keyboard move, so focus rings show (checklist §6).
            window.note_keyboard_input();
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
                    // The panel under the pointer as it is now (a key press
                    // right after coming to the app: GPUI has not seen the
                    // pointer move), else the focused one, else the viewer.
                    let at = self
                        .host
                        .as_ref()
                        .map(|h| {
                            let (x, y) = h.borrow().pointer();
                            point(px(x), px(y))
                        })
                        .unwrap_or_else(|| window.mouse_position());
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
                        })
                        .or_else(|| {
                            self.panel_order
                                .iter()
                                .find(|id| id.as_str() == layout::VIEWER)
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
            // This window was brought forward in its tab group: draw again,
            // which publishes the strip for the new selection.
            "shellTabShown" => {}
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
            "layout:define" => self.define_workspace(&cmd.args),
            "settings" => crate::open_settings(cx),
            "newWindow" => crate::open_editor(cx),
            // One project per window: a new tab is a new window.
            "newTab" => crate::open_editor(cx),
            "deselect" | "cancel" => {
                if self.gesture.is_some() {
                    self.cancel_gesture(cx);
                } else {
                    handled = false;
                }
            }
            name if name.starts_with("workspace:") => match self.workspaces.get(&name[10..]) {
                Some(r) => {
                    let r = r.clone();
                    self.activate_record(r);
                }
                None => self.status = "That workspace no longer exists.".into(),
            },
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
    /// A panel that is not in the workspace (the Agent popover, a window of
    /// its own) dragged over it to `at` (this window's points): the same drop
    /// preview a tab drag shows, for the panel as Pin would first place it.
    /// True when a drop there would dock it.
    pub fn drag_from_outside(&mut self, id: &str, at: (f32, f32), cx: &mut Context<Self>) -> bool {
        let preview = self.outside_target(id, at, cx);
        let original = match self.gesture.take() {
            Some(Gesture::Panel(d)) => d.original,
            _ => self.layout.clone(),
        };
        let pointer = point(px(at.0), px(at.1));
        self.gesture = Some(Gesture::Panel(PanelDrag {
            id: id.into(),
            stack: false,
            start: pointer,
            original,
            at: pointer,
            moved: true,
            preview,
        }));
        cx.notify();
        preview.is_some()
    }
    /// Ends an outside drag as a drop: the panel docks where its preview
    /// showed (the workspace changes as a Pin there would). False, with the
    /// workspace untouched, when no target was lit.
    pub fn drop_from_outside(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(Gesture::Panel(drag)) = self.gesture.take() else {
            return false;
        };
        let Some(preview) = drag.preview.filter(|_| drag.id == id) else {
            cx.notify();
            return false;
        };
        let panels = self.panels(cx);
        let mut next = self.layout.clone();
        let docked = next
            .show_panel(id, &panels)
            .and_then(|_| next.move_panel_to(id, preview.target).or(Ok(())));
        if let Err(error) = docked {
            self.status = error.to_string();
            cx.notify();
            return false;
        }
        self.layout = next;
        self.slides.armed = true;
        self.save();
        cx.notify();
        true
    }
    /// An outside drag let go off any target, or cancelled: no preview, the
    /// workspace as it was.
    pub fn cancel_from_outside(&mut self, cx: &mut Context<Self>) {
        if let Some(Gesture::Panel(drag)) = self.gesture.take() {
            self.layout = drag.original;
            cx.notify();
        }
    }
    /// Where a panel from outside would dock at `at`: the layout with it
    /// shown (as Pin shows it) asked as for a tab drag; over the group Pin
    /// puts it in, that group itself.
    fn outside_target(&self, id: &str, at: (f32, f32), cx: &App) -> Option<DropPreview> {
        let panels = self.panels(cx);
        let mut with = self.layout.clone();
        with.show_panel(id, &panels).ok()?;
        // A floating panel's own drag docks as the original app's did: near
        // a side column's edge or between its panels; over the middle of
        // the workspace it stays floating (`Layout::floating_drop_target`).
        let preview =
            with.floating_drop_target(&self.geometry, self.dock_bounds, at.0, at.1, &self.tokens)?;
        let mut next = with.clone();
        next.move_panel_to(id, preview.target).ok()?;
        Some(preview)
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
    /// Ends the drag in progress as a release does: a panel moved lands
    /// where its preview showed, a resize keeps the sizes it reached.
    fn end_gesture(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.gesture.take() else {
            return;
        };
        self.resize_live = false;
        if let Gesture::Panel(drag) = g
            && drag.moved
        {
            // The original app says what happened: where it went, that it
            // was already there, or that it was no drop zone.
            let panels = self.panels(cx);
            let name = self.moving_name(&drag.id, drag.stack, &panels);
            let message = match drag.preview {
                Some(p) => {
                    let (near, kind) = self.near_name(p.near, &panels);
                    let moved = if drag.stack {
                        self.layout.move_stack_to(&drag.id, p.target)
                    } else {
                        self.layout.move_panel_to(&drag.id, p.target)
                    };
                    match moved {
                        Ok(()) => match kind {
                            "tab" => format!("{name} tabbed with {near}"),
                            "above" => format!("Moved {name} above {near}"),
                            "below" => format!("Moved {name} below {near}"),
                            _ => format!("Moved {name} → {near}"),
                        },
                        Err(err) => err.to_string(),
                    }
                }
                None => match self.layout.drop_outcome(
                    &self.geometry,
                    self.dock_bounds,
                    (f32::from(drag.at.x), f32::from(drag.at.y)),
                    &self.tokens,
                    &drag.id,
                    drag.stack,
                ) {
                    layout::DropOutcome::Here => format!("{name} is already here"),
                    _ => format!("Drop on a panel or dock to move {name}"),
                },
            };
            self.notify_status(message, cx);
        }
        // A panel let go in a new place travels there.
        self.slides.armed = true;
        self.save();
        cx.notify();
    }
    /// While a drag is in progress, the button's real state: a release that
    /// AppKit delivered elsewhere (or not at all) still ends the drag at
    /// once, so a dragged panel never stays in flight (owner feedback W4).
    fn watch_release(&mut self, cx: &mut Context<Self>) {
        if self.host.is_none() {
            return;
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let held = crate::macos::left_button_held();
                let Ok(going) = this.update(cx, |v, cx| {
                    if v.gesture.is_none() {
                        return false;
                    }
                    if !held {
                        v.end_gesture(cx);
                        return false;
                    }
                    true
                }) else {
                    return;
                };
                if !going {
                    return;
                }
            }
        })
        .detach();
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
    pub fn layout_digest(&self, cx: &App) -> serde_json::Value {
        let panels = self.panels(cx);
        let docks: Vec<_> = self
            .layout
            .docks
            .iter()
            .map(|dock| {
                let panels: Vec<_> = dock
                    .groups
                    .iter()
                    .flat_map(|group| {
                        let panels = &panels;
                        group.tabs.iter().enumerate().map(move |(i, id)| {
                            serde_json::json!({"id":id,"title":panels.title(id),
                                "collapsed":dock.collapsed || i != group.active,
                                "flex":group.size.is_none(),"size":group.size})
                        })
                    })
                    .collect();
                serde_json::json!({"id":dock.id,"size":dock.size,"panels":panels})
            })
            .collect();
        let name = workspace::record_name(&self.record);
        serde_json::json!({"workspace":{"id":self.layout.workspace,"name":name},"docks":docks})
    }
    /// set_panel_layout: `docks` become a new workspace (a copy of this one,
    /// named `name`, or `name 2`… when taken) that this window shows and new
    /// projects start with. Returns its id and name.
    pub fn save_agent_layout(
        &mut self,
        name: &str,
        docks: Vec<crate::layout::Dock>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<(String, String)> {
        let base = layout::layout_name(name)?;
        let layout = Layout {
            version: self.layout.version,
            workspace: workspace::new_id(),
            docks,
        };
        layout.validate()?;
        let taken: Vec<String> = self
            .workspaces
            .records
            .iter()
            .map(|r| workspace::record_name(r).to_owned())
            .collect();
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name) {
            name = format!("{base} {n}");
            n += 1;
        }
        let mut record = workspace::write(&layout, &self.record);
        record["id"] = layout.workspace.clone().into();
        record["name"] = name.clone().into();
        record["builtin"] = false.into();
        record["scope"] = "global".into();
        record["projectId"] = serde_json::Value::Null;
        self.activate_record(record);
        let write = workspace::Write::Default(layout.workspace.clone());
        Rc::make_mut(&mut self.workspaces).apply(&write);
        self.outbox.push(write);
        cx.notify();
        Ok((layout.workspace, name))
    }
    /// Runs a shell command without an AppKit host (tests).
    #[doc(hidden)]
    pub fn run_shell_command(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        self.shell_command(name, cx)
    }
    /// Context menu for a panel tab (also its header). The panel's current
    /// dock carries a checkmark.
    /// A panel's menu (right-click on its header or a tab, or its “…”
    /// button), as the original app's: Maximize Panel (or Restore Panel
    /// Size), Add Panel as Tab, then the closing group. The docking moves
    /// this app adds (window edges, a column beside, the top band) stay in
    /// one Dock submenu at the end.
    pub fn tab_menu_entries(&self, id: &str, cx: &App) -> Vec<MenuEntry> {
        let panels = self.panels(cx);
        let current = self
            .layout
            .docks
            .iter()
            .find(|d| d.groups.iter().any(|g| g.tabs.iter().any(|t| t == id)))
            .map(|d| d.id.as_str());
        let stack: Vec<String> = self
            .layout
            .find(id)
            .map(|(d, g, _)| self.layout.docks[d].groups[g].tabs.clone())
            .unwrap_or_default();
        let maximized = self
            .maximized_panel
            .as_deref()
            .is_some_and(|m| stack.iter().any(|t| t == m));
        let mut items = vec![MenuEntry::item(
            if maximized {
                "Restore Panel Size"
            } else {
                "Maximize Panel"
            },
            format!("panel:maximize:{id}"),
        )];
        // Any panel can join this one as a tab: a closed one opens here, an
        // open one moves here. The viewer takes no tabs.
        if layout::stacks(id) {
            let mut addable: Vec<&str> = panels
                .ids()
                .into_iter()
                .filter(|p| layout::stacks(p) && !stack.iter().any(|t| t == p))
                .collect();
            addable.sort_by_key(|p| panels.title(p).to_lowercase());
            if !addable.is_empty() {
                items.push(MenuEntry::submenu(
                    "Add Panel as Tab",
                    addable
                        .into_iter()
                        .map(|p| {
                            MenuEntry::item(panels.title(p), format!("panel:tabwith:{id}:{p}"))
                        })
                        .collect(),
                ));
            }
        }
        let others = stack.iter().any(|t| t != id && layout::closable(t));
        if layout::closable(id) || others {
            items.push(MenuEntry::separator());
        }
        if layout::closable(id) {
            items.push(MenuEntry::item("Close Panel", format!("panel:close:{id}")));
        }
        if others {
            items.push(MenuEntry::item(
                "Close Other Panels in Group",
                format!("panel:closeothers:{id}"),
            ));
        }
        // This app's own docking moves, kept for the menu and the keyboard.
        let mut dock = vec![];
        for (label, edge) in [
            ("Left Edge", "left"),
            ("Right Edge", "right"),
            ("Top Edge", "top"),
            ("Bottom Edge", "bottom"),
        ] {
            dock.push(
                MenuEntry::item(label, format!("panel:dock:{id}:{edge}"))
                    .checked(current == Some(edge)),
            );
        }
        if !matches!(current, Some("bottom" | "top")) {
            dock.push(MenuEntry::separator());
            dock.push(MenuEntry::item(
                "New Column to the Left",
                format!("panel:column:{id}:left"),
            ));
            if current != Some("right") {
                dock.push(MenuEntry::item(
                    "New Column to the Right",
                    format!("panel:column:{id}:right"),
                ));
            }
        }
        items.push(MenuEntry::separator());
        items.push(MenuEntry::submenu("Dock", dock));
        items
    }
    fn tab_menu(&self, id: &str, position: Point<Pixels>, cx: &mut App) {
        let Some(h) = &self.host else { return };
        let at = (f32::from(position.x), f32::from(position.y));
        let entries = self.tab_menu_entries(id, cx);
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
        // Built-ins, then saved workspaces (a quiet separator between).
        let mut items = vec![];
        let mut builtin = true;
        for (id, name, b) in self.workspace_choices() {
            if builtin && !b {
                items.push(MenuEntry::separator());
            }
            builtin = b;
            items.push(
                MenuEntry::item(name, format!("workspace:{id}"))
                    .checked(id == self.layout.workspace),
            );
        }
        items.push(MenuEntry::separator());
        items.push(MenuEntry::item("Save Workspace…", "layout:save"));
        // Was in the panel menu; the original app's panel menu has no
        // workspace items.
        items.push(MenuEntry::item("Reset Workspace", "layout:reset"));
        h.borrow()
            .context_menu(&items, f32::from(position.x), f32::from(position.y));
    }
}
/// A token colour at an alpha (`translucency.*`, `Tokens::edge`, `Tokens::line`).
/// Space between neighbouring tab pills.
const TAB_SPACING: f32 = 2.;
/// Width of a header title at `size` in the medium weight, as the tab strip draws it.
fn title_width(text: &str, size: f32, window: &Window) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: Font {
            family: pm_design::TEXT_FONT.into(),
            weight: FontWeight::MEDIUM,
            ..window.text_style().font()
        },
        color: Hsla::default(),
        background_color: None,
        underline: None,
        strikethrough: None,
        letter_spacing: None,
    };
    let line = window
        .text_system()
        .shape_line(text.to_string().into(), px(size), &[run]);
    f32::from(line.width).ceil()
}
pub use crate::kit::ink as tint;
/// Panels gliding to the places a discrete layout change gave them. Each is
/// laid out at its new size at once and travels from where it was on the
/// layout spring; anything else (a splitter drag, a window resize) places
/// panels where they are with no lag.
#[derive(Default)]
pub struct PanelSlides {
    /// Where each panel was last drawn (its rect at rest).
    last: HashMap<String, Rect>,
    /// Offsets still travelling, by panel.
    moving: HashMap<String, pm_design::motion::Animated<2>>,
    clock: pm_design::motion::Clock,
    /// The next placement is a discrete change: panels that moved slide.
    pub armed: bool,
}
impl PanelSlides {
    /// Takes this frame's rects; true while something still travels.
    pub fn place(&mut self, placed: &[(String, Rect)], t: &Tokens) -> bool {
        let dt = self.clock.tick(std::time::Instant::now());
        if std::mem::take(&mut self.armed) {
            for (id, rect) in placed {
                // The viewer's picture is a Metal layer placed from the
                // panel's rect: it takes its new place at once.
                let Some(old) = self.last.get(id).filter(|_| id != "viewer") else {
                    continue;
                };
                // From where it shows now (a slide already under way keeps going).
                let shown = self.shown(id, *old);
                let offset = [shown.x - rect.x, shown.y - rect.y];
                if offset[0].abs() < 1. && offset[1].abs() < 1. {
                    continue;
                }
                let mut slide = pm_design::motion::Animated::new(
                    offset,
                    pm_design::motion::Spring::SNAP,
                    pm_design::motion::POINT_PRECISION,
                );
                slide.animate_to([0., 0.], t.spring(pm_design::motion::Preset::Layout));
                self.moving.insert(id.clone(), slide);
            }
        } else if placed
            .iter()
            .any(|(id, r)| self.last.get(id).is_some_and(|o| o != r))
        {
            // A live change (a splitter, the window): no lag.
            self.moving.clear();
        }
        self.last = placed.iter().cloned().collect();
        self.moving.retain(|_, slide| slide.step(dt));
        if self.moving.is_empty() {
            self.clock.rest();
        }
        !self.moving.is_empty()
    }
    /// The rect a panel shows at now: its place, plus what is left of its
    /// travel.
    pub fn shown(&self, id: &str, rect: Rect) -> Rect {
        self.moving.get(id).map_or(rect, |slide| Rect {
            x: rect.x + slide.value[0],
            y: rect.y + slide.value[1],
            ..rect
        })
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
/// Centred empty state (language §9/§14): `heading` + one `reading` line.
/// Callers may append one action.
pub fn empty_state(t: &Tokens, heading: &str, line: &str) -> Div {
    crate::kit::empty_state(t, "tray", heading, line)
        .w_full()
        .justify_center()
        .px(px(t.space.xxl))
}

pub use crate::kit::tooltip;
impl Editor {
    /// Home fills the window below the toolbar: no docks, no viewer slot,
    /// no native panel bodies and no status bar (v1's Projects screen).
    /// The title bar's empty band, `top` points tall. AppKit passes a press
    /// on the toolbar's empty space through to this view, so the title bar's
    /// own double-click never runs; a double-click here does what the person
    /// chose in System Settings (Fill, Zoom or Minimize). Single presses fall
    /// through untouched: AppKit still moves the window from here.
    fn titlebar_band(top: f32) -> Div {
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(top))
            .on_mouse_down(MouseButton::Left, |e: &MouseDownEvent, window, _| {
                if e.click_count == 2 {
                    window.titlebar_double_click();
                }
            })
    }
    fn render_home(&mut self, home: AnyElement, top: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        if let Some(h) = &self.host {
            let h = h.borrow();
            h.viewer_rect.set(None);
            // Home covers the viewer: its extra 3D views have no place.
            h.viewer_tiles.borrow_mut().clear();
            h.canvas_focus.set(false);
            h.keyboard_control_focus.set(true);
            h.publish_menu_state(&self.menu_state(cx));
            h.native_panels.borrow_mut().clear();
        }
        if let Some(integration) = &self.integration {
            integration.shown_panels(&[], cx);
        }
        let t = &self.tokens;
        let (ground, ground_alpha) = t.ground();
        // The sidebar column is the system's material (`Host::sync_sidebar`):
        // the ground is painted everywhere else.
        let clear = self
            .host
            .as_ref()
            .map_or(0., |h| h.borrow().sidebar_width());
        div()
            .id("editor")
            .role(Role::Group)
            .accessibility_id("shell.editor")
            .aria_label("Powermove editor")
            .size_full()
            .relative()
            .child(Self::titlebar_band(top))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        window.paint_quad(
                            PaintQuad::new(Bounds::new(
                                point(bounds.origin.x + px(clear), bounds.origin.y),
                                gpui::size(
                                    (bounds.size.width - px(clear)).max(px(0.)),
                                    bounds.size.height,
                                ),
                            ))
                            .background(tint(ground, ground_alpha)),
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                if let Some(i) = &this.integration {
                    i.drop_paths(paths.paths().to_vec(), cx);
                }
            }))
            .child(
                div()
                    .absolute()
                    .top(px(top))
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .child(home),
            )
    }
}
impl Render for Editor {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panels = self.panels(cx);
        if let Some(id) = self.integration.as_ref().and_then(|i| i.project_id(cx))
            && self.project != id
        {
            self.bind_project(id);
        }
        self.sync_workspaces(cx);
        let t = self.tokens.clone();
        let m = t.metrics;
        let c = t.colors.clone();
        let viewport = w.viewport_size();
        let top = self
            .host
            .as_ref()
            .map_or(m.toolbar, |h| h.borrow().toolbar_inset());
        if let Some(home) = self.integration.clone().and_then(|i| i.home(w, cx)) {
            return self.render_home(home, top, cx).into_any_element();
        }
        // The toolbar band already separates the docks from the window's top
        // edge; the other three sides keep the dock gap.
        self.dock_bounds = Rect {
            x: m.dock_gap,
            y: top,
            w: (f32::from(viewport.width) - m.dock_gap * 2.).max(0.),
            h: (f32::from(viewport.height) - top - m.status - m.dock_gap).max(0.),
        };
        self.geometry = self.layout.geometry_with_maximized(
            self.dock_bounds,
            &t,
            self.maximized_panel.as_deref(),
        );
        let placed: Vec<(String, Rect)> = self
            .geometry
            .panels
            .iter()
            .map(|p| {
                let group = &self.layout.docks[p.dock].groups[p.group];
                (group.active().to_string(), p.rect)
            })
            .collect();
        if self.slides.place(&placed, &t) {
            w.request_animation_frame();
        }
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
        // Studio 3D views split the viewer's body into panes; the first is
        // the main viewer, the host draws the others (3d-studio.md §2).
        let viewer = match (&self.host, viewer) {
            (Some(h), Some(r)) if h.borrow().viewer_split.get() > 1 => {
                let tiles =
                    crate::layout::view_tiles(r, h.borrow().viewer_split.get(), m.border * 2.);
                let first = tiles[0];
                *h.borrow().viewer_tiles.borrow_mut() = tiles;
                Some(first)
            }
            (Some(h), _) => {
                h.borrow().viewer_tiles.borrow_mut().clear();
                viewer
            }
            _ => viewer,
        };
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
        let (line, line_alpha) = t.line();
        // Extension pages sit below GPUI too; their bodies are read at paint
        // time, after this frame's layout has recorded them.
        let native_bodies = self.host.as_ref().map(|h| h.borrow().native_panels.clone());
        let dragging = matches!(&self.gesture, Some(Gesture::Panel(d)) if d.moved);
        // A resize keeps its cursor wherever the pointer goes until release.
        let resizing = match &self.gesture {
            Some(Gesture::Resize(d, ..)) if d.horizontal => Some(CursorStyle::ResizeUpDown),
            Some(Gesture::Resize(..)) => Some(CursorStyle::ResizeLeftRight),
            _ => None,
        };
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
            .when_some(resizing, |d, cursor| d.cursor(cursor))
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
                        // The button is up: a release that never reached
                        // this window ends the drag here, where it showed.
                        if this.gesture.is_some() {
                            this.end_gesture(cx);
                        }
                        return;
                    }
                    match &mut this.gesture {
                        Some(Gesture::Resize(d, start, original)) => {
                            let delta = if d.horizontal {
                                f32::from(e.position.y - start.y)
                            } else {
                                f32::from(e.position.x - start.x)
                            };
                            // The glow follows the pointer along the gap.
                            let along = if d.horizontal {
                                f32::from(e.position.x) - d.rect.x
                            } else {
                                f32::from(e.position.y) - d.rect.y
                            };
                            this.split_hover = Some(((d.dock, d.group, d.horizontal), along));
                            // Between two panels the original app waits for
                            // 2 pt before it resizes.
                            if d.group.is_some()
                                && !this.resize_live
                                && delta.abs() < this.tokens.metrics.splitter_dead_zone
                            {
                                cx.notify();
                                return;
                            }
                            this.resize_live = true;
                            this.layout = original.clone();
                            this.layout.resize(d, delta, this.dock_bounds, &this.tokens);
                            cx.notify();
                        }
                        Some(Gesture::Panel(drag)) => {
                            drag.at = e.position;
                            let dx = f32::from(e.position.x - drag.start.x);
                            let dy = f32::from(e.position.y - drag.start.y);
                            drag.moved |= dx.hypot(dy) >= this.tokens.metrics.panel_drag_threshold;
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
                let listener = cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_gesture(cx));
                move |e: &MouseUpEvent, phase, _, w, cx| {
                    if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                        listener(e, w, cx);
                    }
                }
            })
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                // A drag still in flight (its release never came) ends where
                // it showed before this press is handled, never on it.
                if this.gesture.is_some() {
                    this.end_gesture(cx);
                }
                if this.ringed_panel.take().is_some() {
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    this.cancel_gesture(cx);
                    return;
                }
                if let Some(d) = this.split_focus.clone() {
                    let key = e.keystroke.key.as_str();
                    let delta = match key {
                        "left" | "up" => -this.tokens.metrics.keyboard_step,
                        "right" | "down" => this.tokens.metrics.keyboard_step,
                        // Return puts a dock back at its size (the original app).
                        "enter" => {
                            this.layout.reset_dock(&d);
                            0.
                        }
                        _ => return,
                    };
                    this.layout
                        .resize(&d, delta, this.dock_bounds, &this.tokens);
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
        // Where each header's tabs are drawn, for drops over a header.
        let mut tab_spans: HashMap<(usize, usize), Vec<(usize, f32, f32)>> = HashMap::new();
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
            // Folded to its header (a double-click on the header).
            let collapsed = group.collapsed && !is_viewer;
            // Quiet text tabs (language §9): each takes its title's width plus
            // the leading close slot, trailing padding and the pill spacing.
            // The leading close slot is the panel inset, and tab groups start
            // flush, so the first title lands on the inset like a single title.
            let close_slot = m.inset;
            let widths: Vec<f32> = tabs
                .iter()
                .map(|tab| {
                    title_width(panels.title(tab), t.typography.title, w)
                        + close_slot
                        + m.gap
                        + TAB_SPACING
                })
                .collect();
            let mut panel = positioned(self.slides.shown(&id, p.rect))
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
                .aria_label(panels.title(&id).to_string())
                // A panel is a borderless surface: its edge is the elevation's
                // hairline, its corner continuous.
                .corner(&t, m.panel_radius)
                .elevated(&t, pm_design::Elevation::Panel)
                .overflow_hidden();
            if body_native && !collapsed {
                // A native body (an extension's web page) takes the pointer
                // where it shows, so it stops short of the dividers' grab.
                let edge = m.border.max(m.splitter_reach);
                native_bodies.push((
                    id.to_string(),
                    Rect {
                        x: p.rect.x + edge,
                        y: p.rect.y + m.header,
                        w: (p.rect.w - edge * 2.).max(0.),
                        h: (p.rect.h - m.header - edge).max(0.),
                    },
                ));
            } else if self
                .integration
                .as_ref()
                .is_some_and(|i| i.paints_own_ground(&id, cx))
            {
                // Header only: the body paints its own ground (see below).
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
            let own_ground = self
                .integration
                .as_ref()
                .is_some_and(|i| i.paints_own_ground(&id, cx));
            let custom = self
                .integration
                .as_ref()
                .and_then(|i| i.header(&id, p.rect, tabs.len() > 1, w, cx));
            // The header's own tools (Add Panel, or the panel's actions such as
            // Project's search and +) keep their room beside the tabs.
            let tools = if custom.is_some() {
                2. * m.hit + m.gap
            } else {
                m.hit
            };
            // Gaps: trailing padding, strip|chevron and chevron|tools, plus one spare.
            let tab_track = group.tab_track(p.rect.w - m.gap * 4. - tools, &widths, &t);
            let group_name: SharedString = format!("tabs-{}-{}", p.dock, p.group).into();
            let header_id = id.clone();
            let menu_header = id.clone();
            let mut header = div()
                .id(format!("header-{}-{}", p.dock, p.group))
                .role(Role::Group)
                .aria_label(format!("{} header", panels.title(&id)))
                .group(group_name.clone())
                .flex()
                .items_center()
                .gap(px(m.gap))
                .h(px(m.header))
                .flex_none()
                .pl(px(m.inset))
                .pr(px(m.gap))
                // The native viewer body has no panel fill, and the timeline paints
                // its own ground: their headers carry the material.
                .when((is_viewer && native) || own_ground, |h| {
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
                        // A double-click folds the panel to its header, and
                        // opens it again (the original app).
                        if e.click_count >= 2 {
                            this.gesture = None;
                            if this.layout.toggle_collapsed(&header_id) {
                                this.slides.armed = true;
                                this.save();
                            }
                            cx.notify();
                            return;
                        }
                        this.begin_panel_drag(&header_id, e.position, true);
                        this.watch_release(cx);
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
            if multi {
                header = header.pl(px(0.));
            }
            if multi || custom.is_none() {
                // Quiet text tabs (language §9): the active title in `text`, the
                // rest in `muted`; a pill appears only on hover. The whole tab is
                // the drag handle. A single panel shows its title the same way.
                let mut strip = div()
                    .flex()
                    .items_center()
                    .gap(px(TAB_SPACING))
                    .when(multi, |s| s.flex_1().min_w(px(0.)).overflow_hidden());
                let mut span_x = p.rect.x + if multi { 0. } else { m.inset };
                let mut spans = vec![];
                for (ti, tab) in tabs.iter().enumerate() {
                    if !tab_track.visible.contains(&ti) {
                        continue;
                    }
                    {
                        let full = if multi {
                            widths[ti] - TAB_SPACING
                        } else {
                            title_width(panels.title(tab), t.typography.title, w)
                        };
                        let drawn = match tab_track.active_limit.filter(|_| ti == active) {
                            Some(limit) => full.min(limit),
                            None => full,
                        };
                        spans.push((ti, span_x, span_x + drawn));
                        span_x += drawn + TAB_SPACING;
                    }
                    let tabid = tab.clone();
                    let menu_id = tab.clone();
                    let selected = ti == active;
                    let ghost = dragged.as_deref() == Some(tab.as_str());
                    let close_id = tab.clone();
                    let closable = layout::closable(tab) && multi;
                    let tab_group: SharedString = format!("tab-{tab}").into();
                    let title = panels.title(tab).to_string();
                    let mut cell = div()
                        .id(format!("tab-{tab}"))
                        .group(tab_group.clone())
                        .role(Role::Tab)
                        .accessibility_id(format!("shell.tab.{tab}"))
                        .aria_selected(selected)
                        .tab_index(0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.layout.docks[p.dock].groups[p.group].active = ti;
                            this.save();
                            cx.notify();
                        }))
                        .aria_label(title.clone())
                        .relative()
                        .flex_none()
                        .flex()
                        .items_center()
                        .h(px(m.hit))
                        .when(multi, |s| {
                            s.pl(px(close_slot))
                                .pr(px(m.gap))
                                .rounded(px(m.control_radius))
                                .hover_style(|s| s.bg(rgb(c.hover)))
                        })
                        .when(ghost, |s| s.opacity(0.4))
                        .text_color(rgb(if selected || !multi { c.text } else { c.muted }))
                        .when(multi && !selected, |s| {
                            s.group_hover_style(tab_group.clone(), |s| {
                                s.text_color(rgb(c.secondary))
                            })
                        })
                        .text_size(px(t.typography.title))
                        .font_weight(FontWeight(t.typography.medium_weight))
                        .whitespace_nowrap();
                    // Only an active tab wider than the room beside More Panels truncates.
                    cell = match tab_track.active_limit.filter(|_| selected) {
                        Some(limit) => cell
                            .max_w(px(limit))
                            .min_w(px(0.))
                            .overflow_hidden()
                            .child(div().min_w(px(0.)).truncate().child(title.clone())),
                        None => cell.flex_none().child(title.clone()),
                    };
                    if closable {
                        // Mac tabs: the close button sits at the leading edge, on hover.
                        cell = cell.child(
                            div()
                                .id(format!("close-{tab}"))
                                .aria_label(format!("Close {title}"))
                                .role(Role::Button)
                                .tab_index(0)
                                .focus_style(|s| s.opacity(1.))
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(px(close_slot))
                                .flex()
                                .items_center()
                                .justify_center()
                                .opacity(0.)
                                .group_hover_style(tab_group.clone(), |s| s.opacity(1.))
                                .child(crate::symbol::symbol(
                                    "xmark",
                                    m.symbol_inline - 3.,
                                    crate::symbol::Weight::Semibold,
                                    c.muted,
                                    w,
                                ))
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
                                this.watch_release(cx);
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
                tab_spans.insert((p.dock, p.group), spans);
                header = header.child(strip);
                if !tab_track.overflow.is_empty() {
                    let entries: Vec<_> = tab_track
                        .overflow
                        .iter()
                        .map(|&i| {
                            MenuEntry::item(
                                panels.title(&tabs[i]),
                                format!("panel:activate:{}", tabs[i]),
                            )
                            .checked(i == active)
                        })
                        .collect();
                    header = header.child(
                        div()
                            .id(format!("more-panels-{}-{}", p.dock, p.group))
                            .flex_none()
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
                // The original app's “…” button: this panel's menu (Add Panel
                // as Tab is in it).
                let options_group: SharedString = format!("options-{}-{}", p.dock, p.group).into();
                let options_id = id.clone();
                let options_title = format!("{} Options", panels.title(&id));
                header = header.child(
                    div()
                        .id(format!("options-{}-{}", p.dock, p.group))
                        .group(options_group.clone())
                        .role(Role::Button)
                        .aria_label(options_title.clone())
                        .tab_index(tabs.len() as isize)
                        .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
                            this.tab_menu(&options_id, e.position(), cx)
                        }))
                        .tooltip(tooltip(&t, &options_title))
                        .ml_auto()
                        .relative()
                        .size(px(m.hit))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(m.control_radius))
                        .hover_style(|s| s.bg(rgb(c.hover)))
                        .child(crate::symbol::symbol(
                            "ellipsis",
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
                                .group_hover_style(options_group, |s| s.opacity(1.))
                                .child(crate::symbol::symbol(
                                    "ellipsis",
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
            if collapsed {
                root = root.child(panel);
                continue;
            }
            let content = self
                .integration
                .as_ref()
                .and_then(|i| i.panel(&id, p.rect, w, cx));
            if let Some(content) = content {
                panel = panel.child(content);
            } else {
                let unknown = format!("No installed extension provides “{id}”.");
                let (heading, line, action) = if panels.contains(&id) {
                    layout::empty_state(&id)
                } else {
                    ("Panel Unavailable", unknown.as_str(), None)
                };
                let mut empty = empty_state(&t, heading, line)
                    .h(px((p.rect.h - m.header).max(0.)))
                    .overflow_hidden();
                // Actions run only with an app attached (the bare shell has no importer).
                if let Some((label, command)) = action.filter(|_| self.integration.is_some()) {
                    empty = empty.child(
                        div()
                            .id(format!("empty-action-{id}"))
                            .role(Role::Button)
                            .aria_label(label)
                            .tab_index(100)
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
        self.geometry.tabs = tab_spans;
        if let Some(integration) = &self.integration {
            // Every panel on screen, built-in or an extension's: the viewer
            // and the timeline are not native bodies but are shown.
            let shown: Vec<&str> = self.panel_order.iter().map(String::as_str).collect();
            integration.shown_panels(&shown, cx);
        }
        if let Some(h) = &self.host {
            *h.borrow().native_panels.borrow_mut() = native_bodies;
        }
        for (i, d) in self.geometry.dividers.clone().into_iter().enumerate() {
            let divd = d.clone();
            let key_divider = d.clone();
            let hover_divider = d.clone();
            let hover_key = (d.dock, d.group, d.horizontal);
            // The original app's splitter: a short accent glow in the gap
            // that follows the pointer, while hovered or dragged; none while
            // a panel is being moved.
            let resizing_this = matches!(&self.gesture, Some(Gesture::Resize(g, ..))
                if (g.dock, g.group, g.horizontal) == hover_key);
            let glow = self
                .split_hover
                .filter(|(k, _)| *k == hover_key)
                .filter(|_| !dragging && (self.gesture.is_none() || resizing_this))
                .map(|(_, along)| {
                    let reach = m.splitter_reach;
                    let length = if d.horizontal { d.rect.w } else { d.rect.h };
                    let glow_len = m.splitter_glow.min(length);
                    let start = (along - glow_len / 2.).clamp(0., (length - glow_len).max(0.));
                    let thick = m.splitter_glow_width;
                    let across = if d.horizontal { d.rect.h } else { d.rect.w };
                    // Inside the hit area, which reaches `reach` past the gap.
                    let offset = reach + (across - thick) / 2.;
                    let half = |first: bool| {
                        let (from, to) = if first { (0., 1.) } else { (1., 0.) };
                        let angle = if d.horizontal { 90. } else { 180. };
                        div().flex_1().bg(linear_gradient(
                            angle,
                            linear_color_stop(crate::kit::ink(c.accent, from), 0.),
                            linear_color_stop(crate::kit::ink(c.accent, to), 1.),
                        ))
                    };
                    let bar = div()
                        .absolute()
                        .flex()
                        .rounded(px(thick / 2.))
                        .child(half(true))
                        .child(half(false));
                    if d.horizontal {
                        bar.flex_row()
                            .left(px(start))
                            .top(px(offset))
                            .w(px(glow_len))
                            .h(px(thick))
                    } else {
                        bar.flex_col()
                            .top(px(start))
                            .left(px(offset))
                            .h(px(glow_len))
                            .w(px(thick))
                    }
                });
            // As in Final Cut and After Effects, the splitter is the gap
            // itself: nothing is drawn on it, the resize cursor says it can
            // be dragged, along its whole length and a few points into the
            // panels on each side (the gap alone was too thin to find).
            let reach = m.splitter_reach;
            let hit = if d.horizontal {
                Rect {
                    y: d.rect.y - reach,
                    h: d.rect.h + 2. * reach,
                    ..d.rect
                }
            } else {
                Rect {
                    x: d.rect.x - reach,
                    w: d.rect.w + 2. * reach,
                    ..d.rect
                }
            };
            root = root.child(
                positioned(hit)
                    .id(format!("splitter-{i}"))
                    .role(Role::Splitter)
                    .aria_label("Resize dock")
                    .tab_index(1000 + i as isize)
                    // Only a keyboard focus shows (a press focuses it too).
                    .focus_visible_style(|s| s.bg(tint(c.focus, 0.5)))
                    .on_key_down(cx.listener(move |this, e: &KeyDownEvent, _, cx| {
                        let delta = match e.keystroke.key.as_str() {
                            "left" | "up" => -this.tokens.metrics.keyboard_step,
                            "right" | "down" => this.tokens.metrics.keyboard_step,
                            // Return puts a dock back at its size (the original app).
                            "enter" if key_divider.group.is_none() => {
                                this.layout.reset_dock(&key_divider);
                                0.
                            }
                            _ => return,
                        };
                        this.layout
                            .resize(&key_divider, delta, this.dock_bounds, &this.tokens);
                        this.save();
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .cursor(if d.horizontal {
                        CursorStyle::ResizeUpDown
                    } else {
                        CursorStyle::ResizeLeftRight
                    })
                    // The glow follows the pointer along the gap.
                    .on_mouse_move(cx.listener(move |this, e: &MouseMoveEvent, _, cx| {
                        let along = if hover_divider.horizontal {
                            f32::from(e.position.x) - hover_divider.rect.x
                        } else {
                            f32::from(e.position.y) - hover_divider.rect.y
                        };
                        this.split_hover = Some((hover_key, along));
                        cx.notify();
                    }))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if !*hovered
                            && this.split_hover.is_some_and(|(k, _)| k == hover_key)
                            && !matches!(this.gesture, Some(Gesture::Resize(..)))
                        {
                            this.split_hover = None;
                            cx.notify();
                        }
                    }))
                    .children(glow)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            // The grab reaches over the panels' edges: the
                            // header or body under it must not also take the
                            // press (a header would start a panel drag).
                            cx.stop_propagation();
                            this.split_focus = Some(divd.clone());
                            if let Some(h) = &this.host {
                                h.borrow_mut().splitter_focus = true;
                            }
                            // A double-click puts a dock back at its size
                            // (the original app); between panels it does nothing.
                            if e.click_count >= 2 {
                                this.gesture = None;
                                if this.layout.reset_dock(&divd) {
                                    this.slides.armed = true;
                                    this.save();
                                }
                                cx.notify();
                                return;
                            }
                            // A folded panel beside it opens first, so the
                            // drag has room to work with.
                            if this.layout.expand_beside(&divd) {
                                this.save();
                            }
                            this.resize_live = false;
                            this.gesture = Some(Gesture::Resize(
                                divd.clone(),
                                e.position,
                                this.layout.clone(),
                            ));
                            this.watch_release(cx);
                            window.focus(&this.focus, cx);
                            cx.notify();
                        }),
                    ),
            );
        }
        // Drop preview and a card that follows the pointer. Work-surface
        // overlays stay solid tints (no glass below the controls band).
        if let Some(Gesture::Panel(drag)) = &self.gesture
            && drag.moved
        {
            let drag = drag.clone();
            let moving = self.moving_name(&drag.id, drag.stack, &panels);
            if let Some(p) = drag.preview {
                // The original app outlines the panel it lands on or beside.
                let target = match p.near {
                    layout::Near::Tab(d, g)
                    | layout::Near::Above(d, g)
                    | layout::Near::Below(d, g) => self
                        .geometry
                        .panels
                        .iter()
                        .find(|q| (q.dock, q.group) == (d, g))
                        .map(|q| (d, g, q.rect)),
                    _ => None,
                };
                if let Some((_, _, r)) = target {
                    root = root.child(
                        positioned(r)
                            .id("drop-frame")
                            .rounded(px(m.panel_radius))
                            .border(px(m.drop_stroke))
                            .border_color(rgb(c.accent)),
                    );
                }
                // Where the panel lands: a light wash of the accent inside a
                // 2 pt accent outline, so the panels it would cover stay in
                // view (an opaque wash hid them while their hosted controls
                // showed).
                root = root.child(
                    positioned(p.rect)
                        .id("drop-preview")
                        .rounded(px(m.panel_radius))
                        .border(px(m.drop_stroke))
                        .border_color(rgb(c.accent))
                        .bg(tint(c.accent, t.translucency.selected)),
                );
                // Joining a stack: a tab marks its place in the header.
                if let (DropTarget::Tab { index, .. }, Some((d, g, r))) = (p.target, target) {
                    let spans = self.geometry.tabs.get(&(d, g));
                    let x = spans
                        .and_then(|s| s.iter().find(|(i, ..)| *i == index).map(|(_, l, _)| *l))
                        .or_else(|| {
                            spans
                                .and_then(|s| s.last())
                                .map(|(_, _, r)| r + TAB_SPACING)
                        })
                        .unwrap_or(r.x + m.inset);
                    let x = x.min(r.x + r.w - m.hit * 2.).max(r.x);
                    root = root.child(
                        div()
                            .id("drop-tab")
                            .absolute()
                            .left(px(x))
                            .top(px(r.y + (m.header - m.hit) / 2.))
                            .h(px(m.hit))
                            .max_w(px((r.x + r.w - x - m.gap).max(m.hit)))
                            .px(px(m.gap))
                            .flex()
                            .items_center()
                            .rounded(px(m.control_radius))
                            .bg(rgb(c.accent_wash))
                            .border(px(m.border))
                            .border_color(rgb(c.accent))
                            .text_color(rgb(c.text))
                            .text_size(px(t.typography.title))
                            .font_weight(FontWeight(t.typography.medium_weight))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(moving.clone()),
                    );
                }
            }
            // The card: what moves and where it would go.
            let destination = self.destination(&drag, &panels);
            root = root.child(
                div()
                    .id("drag-ghost")
                    .absolute()
                    .left(drag.at.x + px(12.))
                    .top(drag.at.y + px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(m.border))
                    .px(px(m.gap + 2.))
                    .py(px(m.gap - 2.))
                    .rounded(px(m.control_radius))
                    .bg(rgb(c.floating))
                    .elevated(&t, pm_design::Elevation::Menu)
                    .shadow_md()
                    .child(
                        div()
                            .font_weight(FontWeight(t.typography.semibold_weight))
                            .whitespace_nowrap()
                            .child(moving),
                    )
                    .child(
                        div()
                            .text_size(px(t.typography.minimum))
                            .text_color(rgb(c.secondary))
                            .whitespace_nowrap()
                            .child(destination),
                    ),
            );
        }
        let workspace = workspace::record_name(&self.record).to_owned();
        // Status bar (language §2): quiet facts. Left: document facts (or a
        // message); right: the workspace pop-up.
        let left = self
            .integration
            .as_ref()
            .map_or_else(|| self.status.clone(), |i| i.status(cx));
        let left = if let Some((notice, _)) = &self.notice {
            notice.clone()
        } else if left.is_empty() {
            self.status.clone()
        } else {
            left
        };
        root = root.child(Self::titlebar_band(top));
        if let Some(toast) = self.integration.as_ref().and_then(|i| i.action_toast(cx)) {
            root = root.child(toast);
        }
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
            .children(
                self.integration
                    .as_ref()
                    .map(|i| i.status_alert(cx))
                    .filter(|s| !s.is_empty())
                    .map(|alert| {
                        div()
                            .id("status-alert")
                            .accessibility_id("shell.status.alert")
                            .role(Role::Button)
                            .aria_label(alert.clone())
                            .flex_none()
                            .px(px(m.gap))
                            .h(px(m.hit))
                            .flex()
                            .items_center()
                            .rounded(px(m.control_radius))
                            .text_color(rgb(c.danger))
                            .hover_style(|s| s.bg(rgb(c.hover)))
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                if let Some(i) = this.integration.clone() {
                                    i.status_alert_pressed(window, cx);
                                }
                            }))
                            .child(alert)
                    }),
            )
            .children(
                self.integration
                    .as_ref()
                    .map(|i| i.status_trailing(cx))
                    .filter(|s| !s.is_empty())
                    .map(|facts| {
                        div()
                            .flex_none()
                            .font_features(FontFeatures(std::sync::Arc::new(vec![(
                                "tnum".into(),
                                1,
                            )])))
                            .child(facts)
                    }),
            )
            .child(
                div()
                    .id("status-workspace")
                    .track_focus(&self.workspace_focus)
                    .tab_index(2000)
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
                    // A control among the bar's quiet facts: the primary
                    // ink, so it never reads as a disabled one.
                    .text_color(rgb(c.text))
                    .hover_style(|s| s.bg(rgb(c.hover)))
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
        .into_any_element()
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

    /// At launch an extension's panels park while its realm starts, then come
    /// back: in their places, with the stack still showing what it showed.
    /// A window whose extensions contribute these panels.
    struct Contributions(Vec<(String, String)>);
    impl crate::integration::Integration for Contributions {
        fn observe(&self, _: &mut Context<Editor>) {}
        fn command(&self, _: &Command, _: &mut Window, _: &mut App) -> bool {
            false
        }
        fn panel(
            &self,
            _: &str,
            _: crate::layout::Rect,
            _: &mut Window,
            _: &mut App,
        ) -> Option<AnyElement> {
            None
        }
        fn contributed_panels(&self, _: &App) -> Vec<(String, String)> {
            self.0.clone()
        }
        fn project_id(&self, _: &App) -> Option<String> {
            None
        }
        fn status(&self, _: &App) -> String {
            String::new()
        }
        fn drop_paths(&self, _: Vec<PathBuf>, _: &mut App) {}
    }

    /// At launch an extension's panels park while its realm starts, then come
    /// back: in their places, with the stack still showing what it showed.
    #[::core::prelude::v1::test]
    fn returning_extension_panels_keep_the_stack_showing_its_tab() {
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
            .update(&mut cx, |v, _, cx| {
                v.integration = Some(Rc::new(Contributions(vec![
                    ("ext.pexels".into(), "Pexels".into()),
                    ("ext.voice".into(), "Voiceover".into()),
                ])));
                // Saved layout: both extension tabs ahead of Properties, which shows.
                for id in ["ext.pexels", "ext.voice"] {
                    assert!(v.shell_command(&format!("panel:show:{id}"), cx));
                    assert!(v.shell_command(&format!("panel:group:{id}:inspector"), cx));
                }
                let (d, g, _) = v.layout.find("inspector").unwrap();
                v.layout.docks[d].groups[g].active = v.layout.find("inspector").unwrap().2;
                let showing = |v: &Editor| {
                    let stack = &v.layout.docks[d].groups[g];
                    stack.tabs[stack.active].clone()
                };
                assert_eq!(showing(v), "inspector");
                for id in ["ext.pexels", "ext.voice"] {
                    assert!(v.shell_command(&format!("panel:park:{id}"), cx));
                    assert_eq!(showing(v), "inspector", "parking {id}");
                }
                for id in ["ext.voice", "ext.pexels"] {
                    assert!(v.shell_command(&format!("panel:show:{id}"), cx));
                    assert!(v.layout.find(id).is_some());
                    assert_eq!(
                        showing(v),
                        "inspector",
                        "{id} came back without taking the tab"
                    );
                }
                // Asking for a panel still shows it.
                assert!(v.shell_command("panel:show:ext.pexels", cx));
                assert_eq!(showing(v), "ext.pexels");
            })
            .unwrap();
    }
    /// A project whose saved stack shows an extension's panel that is not
    /// there at open (uninstalled, or its realm still starting): the window
    /// falls back to another tab, but the workspace keeps the saved choice.
    #[::core::prelude::v1::test]
    fn a_parked_showing_tab_stays_the_saved_choice() {
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
            .update(&mut cx, |v, _, cx| {
                v.integration = Some(Rc::new(Contributions(vec![(
                    "ext.voice".into(),
                    "Voiceover".into(),
                )])));
                assert!(v.shell_command("panel:show:ext.voice", cx));
                assert!(v.shell_command("panel:group:ext.voice:inspector", cx));
                assert!(v.shell_command("panel:show:ext.voice", cx));
                v.save();
                let saved = v.record.clone();
                let showing = |l: &crate::layout::Layout| {
                    let (d, g, _) = l.find("inspector").unwrap();
                    let stack = &l.docks[d].groups[g];
                    stack.tabs[stack.active].clone()
                };
                assert_eq!(showing(&workspace::layout(&saved)), "ext.voice");
                assert!(v.shell_command("panel:park:ext.voice", cx));
                assert_eq!(showing(&v.layout), "inspector", "the window falls back");
                v.save();
                assert_eq!(
                    showing(&workspace::layout(&v.record)),
                    "ext.voice",
                    "the saved stack still shows the parked panel"
                );
                assert_eq!(v.record, saved, "nothing new is written");
            })
            .unwrap();
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
                    // The shell handled the key: rings still show (focus-visible).
                    assert!(w.last_input_was_keyboard(), "Tab is keyboard input");
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    // Not a glob: gpui's own `test` attribute would shadow the standard one.
    use super::{Editor, Rc, Tokens, workspace};
    use gpui::{AnyElement, App, TestAppContext, Window};
    use std::cell::RefCell;

    /// The kernel's store as a window reads it, and what the window writes.
    #[derive(Default)]
    struct Kernel {
        store: RefCell<Rc<workspace::Store>>,
        writes: RefCell<Vec<workspace::Write>>,
    }
    struct Stub(Rc<Kernel>);
    impl crate::integration::Integration for Stub {
        fn observe(&self, _: &mut gpui::Context<Editor>) {}
        fn command(&self, _: &crate::native::Command, _: &mut Window, _: &mut App) -> bool {
            false
        }
        fn panel(
            &self,
            _: &str,
            _: crate::layout::Rect,
            _: &mut Window,
            _: &mut App,
        ) -> Option<AnyElement> {
            None
        }
        fn workspaces(&self, _: &App) -> Option<Rc<workspace::Store>> {
            Some(self.0.store.borrow().clone())
        }
        fn write_workspace(&self, write: workspace::Write, _: &mut App) {
            self.0.writes.borrow_mut().push(write);
        }
        fn project_id(&self, _: &App) -> Option<String> {
            Some("p".into())
        }
        fn status(&self, _: &App) -> String {
            String::new()
        }
        fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    }

    #[gpui::test]
    fn a_workspace_deleted_by_another_window_moves_this_one_to_the_fallback(
        cx: &mut TestAppContext,
    ) {
        let mut mine = workspace::preset("edit").unwrap();
        mine["id"] = "mine".into();
        mine["name"] = "Mine".into();
        mine["builtin"] = false.into();
        let kernel = Rc::new(Kernel::default());
        *kernel.store.borrow_mut() = Rc::new(workspace::Store {
            records: workspace::seeded(std::slice::from_ref(&mine)),
            current: Some("mine".into()),
            default: None,
            project: Some(mine.clone()),
            project_id: Some("p".into()),
            trashed: vec![],
        });
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let stub = kernel.clone();
        let window = cx.add_window(|_, cx| {
            let mut e = Editor::new(tokens, None, cx);
            e.integration = Some(Rc::new(Stub(stub)));
            e
        });
        window
            .update(cx, |v, _, cx| {
                v.sync_workspaces(cx);
                assert_eq!(v.layout.workspace, "mine");
                // Another window deletes it; this project still holds it.
                let before = (**kernel.store.borrow()).clone();
                *kernel.store.borrow_mut() = Rc::new(workspace::Store {
                    records: workspace::seeded(&[]),
                    trashed: vec!["mine".into()],
                    ..before
                });
                kernel.writes.borrow_mut().clear();
                v.sync_workspaces(cx);
                assert_eq!(v.layout.workspace, workspace::FALLBACK);
                // The next frame sends it.
                v.sync_workspaces(cx);
                let writes = kernel.writes.borrow();
                assert!(
                    matches!(&writes[..], [workspace::Write::Active { after, .. }]
                        if workspace::record_id(after) == Some(workspace::FALLBACK)),
                    "the project should keep the fallback: {writes:?}"
                );
            })
            .unwrap();
    }

    /// The Agent popover dragged over the workspace by its own header: as
    /// the original app's floating panel, it docks only near a side column
    /// (its edge, between its panels) with the same drop preview a tab drag
    /// shows; over the middle it stays floating; a cancel changes nothing.
    #[gpui::test]
    fn a_panel_dragged_in_from_outside_docks_where_its_preview_shows(cx: &mut TestAppContext) {
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        view.update(cx, |v, _| {
            if v.layout.find("agent").is_some() {
                v.layout.close_panel("agent").unwrap();
            }
        });
        cx.run_until_parked();
        let before = view.read_with(cx, |v, _| v.layout.clone());
        // 1. Over the middle of the viewer: nothing lit, it stays floating.
        let viewer = view.read_with(cx, |v, _| {
            let (d, g, _) = v.layout.find("viewer").unwrap();
            v.geometry
                .panels
                .iter()
                .find(|p| (p.dock, p.group) == (d, g))
                .unwrap()
                .rect
        });
        let middle = (viewer.x + viewer.w / 2., viewer.y + viewer.h / 2.);
        assert!(!view.update(cx, |v, cx| v.drag_from_outside("agent", middle, cx)));
        assert!(!view.update(cx, |v, cx| v.drop_from_outside("agent", cx)));
        view.read_with(cx, |v, _| assert_eq!(v.layout, before));
        // 2. Somewhere along a side column: lit, and it docks in that column.
        let bounds = view.read_with(cx, |v, _| v.dock_bounds);
        let mut lit = None;
        'find: for i in 0..=96 {
            for j in 0..=48 {
                let at = (
                    bounds.x + bounds.w * i as f32 / 96.,
                    bounds.y + bounds.h * j as f32 / 48.,
                );
                if view.update(cx, |v, cx| v.drag_from_outside("agent", at, cx)) {
                    lit = Some(at);
                    break 'find;
                }
            }
        }
        assert!(lit.is_some(), "a side column takes the floating panel");
        assert!(view.update(cx, |v, cx| v.drop_from_outside("agent", cx)));
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            let (dock, _, _) = v.layout.find("agent").expect("docked");
            assert!(
                matches!(v.layout.docks[dock].id.as_str(), "left" | "right"),
                "in a side column, as the original app docks a floating panel"
            );
        });
        // 3. A cancelled drag leaves the workspace as it was.
        view.update(cx, |v, cx| {
            v.layout = before.clone();
            cx.notify();
        });
        cx.run_until_parked();
        view.update(cx, |v, cx| {
            v.drag_from_outside("agent", lit.unwrap(), cx);
            v.cancel_from_outside(cx);
        });
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            assert_eq!(v.layout, before, "a cancelled drag leaves the workspace");
        });
    }

    /// A drag whose release never reaches the window (AppKit delivered it
    /// elsewhere, owner feedback W4) ends at the next sign the button is up:
    /// a move with no button lands the panel where its preview showed, and a
    /// new press lands it first and then does its own work.
    #[gpui::test]
    fn a_drag_whose_release_is_lost_still_lands_and_never_eats_the_next_press(
        cx: &mut TestAppContext,
    ) {
        use gpui::{Modifiers, MouseButton, point, px};
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        let none = Modifiers::none();
        let header = |v: &Editor, id: &str| {
            let (d, g, _) = v.layout.find(id).unwrap();
            let r = v
                .geometry
                .panels
                .iter()
                .find(|p| (p.dock, p.group) == (d, g))
                .unwrap()
                .rect;
            (r.x + r.w * 0.6, r.y + v.tokens.metrics.header / 2.)
        };
        let drag = |cx: &mut gpui::VisualTestContext, from: (f32, f32), to: (f32, f32)| {
            cx.simulate_mouse_move(point(px(from.0), px(from.1)), None, none);
            cx.simulate_mouse_down(point(px(from.0), px(from.1)), MouseButton::Left, none);
            for s in 1..=8 {
                let t = s as f32 / 8.;
                let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
                cx.simulate_mouse_move(point(px(at.0), px(at.1)), MouseButton::Left, none);
            }
        };
        // 1. Properties dragged onto the viewer's right edge; no release.
        let before = view.read_with(cx, |v, _| v.layout.clone());
        let from = view.read_with(cx, |v, _| header(v, "inspector"));
        let viewer = view.read_with(cx, |v, _| {
            let (d, g, _) = v.layout.find("viewer").unwrap();
            v.geometry
                .panels
                .iter()
                .find(|p| (p.dock, p.group) == (d, g))
                .unwrap()
                .rect
        });
        let to = (viewer.x + viewer.w * 0.92, viewer.y + viewer.h * 0.6);
        drag(cx, from, to);
        let previewed = view.read_with(cx, |v, _| match &v.gesture {
            Some(super::Gesture::Panel(d)) => d.preview.map(|p| p.target),
            _ => None,
        });
        assert!(previewed.is_some(), "the drag shows where it will land");
        // The pointer moves on with the button up: the panel lands there.
        cx.simulate_mouse_move(point(px(to.0 - 20.), px(to.1)), None, none);
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none(), "the drag ended");
            assert_ne!(v.layout, before, "the panel landed");
            assert!(v.layout.find("inspector").is_some());
        });
        // 2. Again, and the next thing is a press on a splitter: the panel
        // lands first and the press starts the splitter's own drag.
        let landed = view.read_with(cx, |v, _| v.layout.clone());
        let from = view.read_with(cx, |v, _| header(v, "fxbrowser"));
        // Onto the left dock's middle: it joins that stack.
        let left = view.read_with(cx, |v, _| v.geometry.panels[0].rect);
        drag(cx, from, (left.x + left.w / 2., left.y + left.h / 2.));
        assert!(
            view.read_with(cx, |v, _| matches!(
                &v.gesture,
                Some(super::Gesture::Panel(d)) if d.preview.is_some()
            )),
            "the second drag shows where it will land"
        );
        let divider = view.read_with(cx, |v, _| {
            v.geometry
                .dividers
                .iter()
                .find(|d| !d.horizontal)
                .unwrap()
                .rect
        });
        let at = point(
            px(divider.x + divider.w / 2.),
            px(divider.y + divider.h / 2.),
        );
        cx.simulate_mouse_down(at, MouseButton::Left, none);
        view.read_with(cx, |v, _| {
            assert_ne!(v.layout, landed, "the stale drag landed before the press");
            assert!(
                matches!(v.gesture, Some(super::Gesture::Resize(..))),
                "the press is the splitter's own"
            );
        });
        cx.simulate_mouse_up(at, MouseButton::Left, none);
        // 3. A splitter drag whose release is lost keeps the size it reached.
        let divider = view.read_with(cx, |v, _| {
            v.geometry
                .dividers
                .iter()
                .find(|d| !d.horizontal)
                .unwrap()
                .rect
        });
        let x = divider.x + divider.w / 2.;
        let y = divider.y + divider.h / 2.;
        let widths = |v: &Editor| v.layout.docks.iter().map(|d| d.size).collect::<Vec<_>>();
        let start = view.read_with(cx, |v, _| widths(v));
        cx.simulate_mouse_move(point(px(x), px(y)), None, none);
        cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Left, none);
        cx.simulate_mouse_move(point(px(x - 60.), px(y)), MouseButton::Left, none);
        let reached = view.read_with(cx, |v, _| widths(v));
        assert_ne!(reached, start, "the splitter moved while held");
        cx.simulate_mouse_move(point(px(x - 40.), px(y + 30.)), None, none);
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none(), "the resize ended");
            assert_eq!(widths(v), reached, "it kept the size it reached");
        });
    }

    /// A splitter shows the resize cursor under the pointer, along its whole
    /// length and a little past each side of the gap, and keeps it for the
    /// whole drag, wherever the pointer goes (owner feedback W1).
    /// U1: Maximize Panel (`) with the pointer over no panel and nothing
    /// focused still maximizes something: the viewer; pressed again, it
    /// restores.
    #[gpui::test]
    fn maximize_panel_falls_back_to_the_viewer(cx: &mut TestAppContext) {
        use gpui::{Modifiers, point, px};
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        // Over the window's title band: no panel there.
        cx.simulate_mouse_move(point(px(4.), px(4.)), None, Modifiers::none());
        cx.update(|w, cx| {
            view.update(cx, |v, cx| {
                v.command(
                    crate::Command {
                        name: "maximizePanel".into(),
                        args: vec![],
                        token: None,
                    },
                    w,
                    cx,
                );
                assert_eq!(v.maximized_panel.as_deref(), Some(crate::layout::VIEWER));
                v.command(
                    crate::Command {
                        name: "maximizePanel".into(),
                        args: vec![],
                        token: None,
                    },
                    w,
                    cx,
                );
                assert_eq!(v.maximized_panel, None);
            })
        });
    }

    #[gpui::test]
    fn every_divider_grabs_along_its_whole_length_in_every_workspace(cx: &mut TestAppContext) {
        use gpui::{CursorStyle, Modifiers, point, px, size};
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let m = tokens.metrics;
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.update(|window, _| window.activate());
        let none = Modifiers::none();
        let reach = m.splitter_reach;
        // Where a divider grabs: its gap and `reach` into each side.
        let grab = |d: &crate::layout::Divider| {
            let r = d.rect;
            if d.horizontal {
                crate::layout::Rect {
                    y: r.y - reach,
                    h: r.h + 2. * reach,
                    ..r
                }
            } else {
                crate::layout::Rect {
                    x: r.x - reach,
                    w: r.w + 2. * reach,
                    ..r
                }
            }
        };
        let mut checked = 0;
        for (w, h) in [(1024., 640.), (1440., 900.), (2560., 1440.)] {
            cx.simulate_resize(size(px(w), px(h)));
            for id in crate::workspace::BUILTIN {
                view.update(cx, |v, cx| {
                    let record = v.workspaces.get(id).cloned().expect(id);
                    v.activate_record(record);
                    cx.notify();
                });
                cx.run_until_parked();
                let dividers = view.read_with(cx, |v, _| v.geometry.dividers.clone());
                for (i, d) in dividers.iter().enumerate() {
                    let wanted = if d.horizontal {
                        CursorStyle::ResizeUpDown
                    } else {
                        CursorStyle::ResizeLeftRight
                    };
                    let r = d.rect;
                    let (length, thick) = if d.horizontal { (r.w, r.h) } else { (r.h, r.w) };
                    // Along its whole length, across its whole grab: the
                    // gap's centre and just inside each outer edge.
                    let mut at = 1.;
                    while at <= length - 1. {
                        for across in [-reach + 0.5, thick / 2., thick + reach - 0.5] {
                            let (x, y) = if d.horizontal {
                                (r.x + at, r.y + across)
                            } else {
                                (r.x + across, r.y + at)
                            };
                            // Where two dividers' grabs cross, either answers.
                            let crossing = dividers
                                .iter()
                                .enumerate()
                                .any(|(j, o)| j != i && grab(o).contains(x, y));
                            if crossing {
                                continue;
                            }
                            cx.simulate_mouse_move(point(px(x), px(y)), None, none);
                            assert_eq!(
                                cx.cursor_style(),
                                wanted,
                                "{id} at {w}x{h}: {d:?} at {x},{y}"
                            );
                            checked += 1;
                        }
                        at += 12.;
                    }
                    // A press at the grab's outer edge, mid-length, starts
                    // resizing this divider (not a panel drag or a click).
                    let (x, y) = if d.horizontal {
                        (r.x + length / 2., r.y + thick + reach - 0.5)
                    } else {
                        (r.x + thick + reach - 0.5, r.y + length / 2.)
                    };
                    if dividers
                        .iter()
                        .enumerate()
                        .any(|(j, o)| j != i && grab(o).contains(x, y))
                    {
                        continue;
                    }
                    cx.simulate_mouse_move(point(px(x), px(y)), None, none);
                    cx.simulate_mouse_down(point(px(x), px(y)), gpui::MouseButton::Left, none);
                    let pressed = view.read_with(cx, |v, _| match &v.gesture {
                        Some(super::Gesture::Resize(g, ..)) => g.rect == d.rect,
                        _ => false,
                    });
                    cx.simulate_mouse_up(point(px(x), px(y)), gpui::MouseButton::Left, none);
                    assert!(pressed, "{id} at {w}x{h}: a press did not grab {d:?}");
                }
            }
        }
        assert!(checked > 1000, "{checked} points");
    }

    #[gpui::test]
    fn splitters_show_the_resize_cursor_and_keep_it_while_dragging(cx: &mut TestAppContext) {
        use gpui::{CursorStyle, Modifiers, MouseButton, point, px};
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        // The headless window owns the cursor once it is active (as on macOS).
        cx.update(|window, _| window.activate());
        cx.run_until_parked();
        let dividers = view.read_with(cx, |v, _| v.geometry.dividers.clone());
        assert!(dividers.len() >= 3, "{dividers:?}");
        let none = Modifiers::none();
        for d in &dividers {
            let r = d.rect;
            let wanted = if d.horizontal {
                CursorStyle::ResizeUpDown
            } else {
                CursorStyle::ResizeLeftRight
            };
            // Middle and both ends of its length; the gap's centre and a
            // point just past its edge.
            let along: Vec<(f32, f32)> = if d.horizontal {
                vec![
                    (r.x + r.w / 2., r.y + r.h / 2.),
                    (r.x + 30., r.y + r.h / 2.),
                    (r.x + r.w - 30., r.y + r.h / 2.),
                    (r.x + r.w / 2., r.y - 1.),
                ]
            } else {
                vec![
                    (r.x + r.w / 2., r.y + r.h / 2.),
                    (r.x + r.w / 2., r.y + 30.),
                    (r.x + r.w / 2., r.y + r.h - 30.),
                    (r.x + r.w + 1., r.y + r.h / 2.),
                ]
            };
            for (x, y) in along {
                cx.simulate_mouse_move(point(px(x), px(y)), None, none);
                assert_eq!(cx.cursor_style(), wanted, "{d:?} at {x},{y}");
            }
        }
        // Inside a panel, away from every gap, the cursor is the arrow.
        let panel = view.read_with(cx, |v, _| v.geometry.panels[0].rect);
        cx.simulate_mouse_move(
            point(px(panel.x + panel.w / 2.), px(panel.y + panel.h / 2.)),
            None,
            none,
        );
        assert_eq!(cx.cursor_style(), CursorStyle::Arrow, "inside a panel");
        // Dragging the first splitter far into a panel keeps its cursor.
        let d = &dividers[0];
        let (x, y) = (d.rect.x + d.rect.w / 2., d.rect.y + d.rect.h / 2.);
        cx.simulate_mouse_move(point(px(x), px(y)), None, none);
        cx.simulate_mouse_down(point(px(x), px(y)), MouseButton::Left, none);
        let away = if d.horizontal {
            (x + 40., y + 120.)
        } else {
            (x + 120., y + 40.)
        };
        cx.simulate_mouse_move(point(px(away.0), px(away.1)), MouseButton::Left, none);
        let wanted = if d.horizontal {
            CursorStyle::ResizeUpDown
        } else {
            CursorStyle::ResizeLeftRight
        };
        assert_eq!(cx.cursor_style(), wanted, "while dragging");
        cx.simulate_mouse_up(point(px(away.0), px(away.1)), MouseButton::Left, none);
    }

    #[gpui::test]
    fn deleting_the_active_workspace_moves_to_the_fallback_and_never_refiles_it(
        cx: &mut TestAppContext,
    ) {
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let window = cx.add_window(|_, cx| Editor::new(tokens, None, cx));
        window
            .update(cx, |v, _, cx| {
                let mut mine = workspace::preset("edit").unwrap();
                mine["id"] = "mine".into();
                mine["name"] = "Mine".into();
                mine["builtin"] = false.into();
                v.workspaces = Rc::new(workspace::Store {
                    records: workspace::seeded(std::slice::from_ref(&mine)),
                    current: Some("mine".into()),
                    default: None,
                    project: Some(mine.clone()),
                    project_id: Some("p".into()),
                    trashed: vec![],
                });
                v.show_record(mine);
                v.outbox.clear();
                assert!(v.run_shell_command("layout:delete:mine", cx));
                assert_eq!(v.layout.workspace, workspace::FALLBACK);
                assert!(
                    v.workspaces.get("mine").is_none(),
                    "{:?}",
                    v.workspaces.records
                );
                assert_eq!(
                    workspace::record_id(&v.workspaces.active(None)),
                    Some(workspace::FALLBACK)
                );
                let archived = v
                    .outbox
                    .iter()
                    .position(|w| *w == workspace::Write::Archive("mine".into()))
                    .expect("archived");
                assert!(
                    v.outbox[archived..].iter().all(|w| match w {
                        workspace::Write::Active { after, .. }
                        | workspace::Write::Upsert(after) =>
                            workspace::record_id(after) != Some("mine"),
                        _ => true,
                    }),
                    "the deleted record was written again: {:?}",
                    v.outbox
                );
            })
            .unwrap();
    }

    /// What the window says is on screen each frame.
    struct Shown(Rc<RefCell<Vec<Vec<String>>>>);
    impl crate::integration::Integration for Shown {
        fn observe(&self, _: &mut gpui::Context<Editor>) {}
        fn command(&self, _: &crate::native::Command, _: &mut Window, _: &mut App) -> bool {
            false
        }
        fn panel(
            &self,
            _: &str,
            _: crate::layout::Rect,
            _: &mut Window,
            _: &mut App,
        ) -> Option<AnyElement> {
            None
        }
        fn shown_panels(&self, shown: &[&str], _: &mut App) {
            let shown = shown.iter().map(|id| (*id).to_owned()).collect();
            self.0.borrow_mut().push(shown);
        }
        fn project_id(&self, _: &App) -> Option<String> {
            None
        }
        fn status(&self, _: &App) -> String {
            String::new()
        }
        fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    }

    /// The viewer and the timeline are on screen even though no extension
    /// draws them: the app keeps their picture place, playhead and timecode
    /// only for panels the window names as shown (it named only extension
    /// bodies, so the playhead never drew). A maximised panel shows alone.
    #[gpui::test]
    fn the_viewer_and_timeline_are_named_shown_until_another_panel_is_maximised(
        cx: &mut TestAppContext,
    ) {
        let frames = Rc::new(RefCell::new(vec![]));
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let record = frames.clone();
        let window = cx.add_window(|_, cx| {
            let mut e = Editor::new(tokens, None, cx);
            e.integration = Some(Rc::new(Shown(record)));
            e
        });
        cx.run_until_parked();
        let last = || frames.borrow().last().cloned().expect("a frame");
        let shown = last();
        assert!(
            shown.iter().any(|id| id == "viewer") && shown.iter().any(|id| id == "timeline"),
            "{shown:?}"
        );
        let other = window
            .update(cx, |v, _, _| {
                v.layout
                    .open_panels()
                    .into_iter()
                    .find(|id| !matches!(*id, "viewer" | "timeline"))
                    .map(str::to_owned)
            })
            .unwrap()
            .expect("a third panel");
        window
            .update(cx, |v, _, cx| {
                assert!(v.run_shell_command(&format!("panel:maximize:{other}"), cx));
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(last(), [other]);
    }
}

#[cfg(test)]
mod slide_tests {
    use super::{PanelSlides, Rect};
    fn r(x: f32, y: f32) -> Rect {
        Rect {
            x,
            y,
            w: 300.,
            h: 200.,
        }
    }
    /// A drop moves a panel from where it was to its new place on the layout
    /// spring; a live change (a splitter, the window) places it at once.
    #[::core::prelude::v1::test]
    fn a_moved_panel_slides_and_a_live_change_does_not() {
        let t = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
        let mut s = PanelSlides::default();
        s.place(&[("project".into(), r(8., 52.))], &t);
        // Dropped on the other side: it starts where it was.
        s.armed = true;
        assert!(s.place(&[("project".into(), r(1112., 52.))], &t));
        let shown = s.shown("project", r(1112., 52.));
        assert!(shown.x < 1100., "still near where it was: {}", shown.x);
        // A splitter drag moves it again: no lag, the slide stops.
        assert!(!s.place(&[("project".into(), r(1100., 52.))], &t));
        assert_eq!(s.shown("project", r(1100., 52.)), r(1100., 52.));
        // The viewer never slides (its picture is a Metal layer).
        s.place(&[("viewer".into(), r(316., 52.))], &t);
        s.armed = true;
        s.place(&[("viewer".into(), r(8., 52.))], &t);
        assert_eq!(s.shown("viewer", r(8., 52.)), r(8., 52.));
    }
}
