use crate::kit::Recipes as _;
use crate::{
    layout::{self, Divider, DropPreview, DropTarget, Edge, Geometry, Layout, Rect},
    native::{Command, Host, MenuEntry, MenuState},
    session::WindowState,
    workspace,
};
#[cfg(test)]
use ::core::prelude::v1::test;
use gpui::{prelude::*, *};
use pm_design::Tokens;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

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
    /// The target the window shows the layout of: `preview`, once the
    /// pointer has rested on it (`drag::rested`). A release never waits
    /// for it; it lands on `preview`.
    shown: Option<DropPreview>,
    /// Where and when the pointer last came to rest.
    rest: ((f32, f32), std::time::Instant),
    /// The panel is in the layout: the window shows the layout a drop
    /// would give while the drag lasts (`Editor::retarget`). False for a
    /// panel dragged in from outside, which shows its landing place only.
    live: bool,
    /// The geometry targets are read from: as drawn at pick-up, so no zone
    /// moves under the pointer when the layout re-flows to the preview.
    geometry: Geometry,
    /// Each column's scroll at pick-up (a cancel returns to it) ...
    scroll: HashMap<String, f32>,
    /// ... and how far the drag's own edge scroll has moved each since.
    scrolled: HashMap<String, f32>,
    /// When the press became a drag: the chip fades in from then.
    lifted: Option<std::time::Instant>,
    /// When the edge scroll last ran.
    scroll_tick: Option<std::time::Instant>,
}
/// After a release: the chip gliding to where the panel landed (or back to
/// where it was picked up) and the landing slot's wash fading.
#[derive(Clone)]
struct Settle {
    name: String,
    from: (f32, f32),
    to: (f32, f32),
    /// The panel whose slot fades (a drop on a column position).
    slot: Option<String>,
    since: std::time::Instant,
}
#[derive(Clone)]
struct AreaDrag {
    id: String,
    corner: crate::area::Corner,
    start: Point<Pixels>,
    at: Point<Pixels>,
    original: Layout,
    moved: bool,
    action: crate::area::Action,
}
#[derive(Clone)]
enum Gesture {
    Area(AreaDrag),
    Resize(Divider, Point<Pixels>, Layout),
    Panel(PanelDrag),
}
impl Gesture {
    fn original(self) -> Layout {
        match self {
            Gesture::Resize(_, _, l) => l,
            Gesture::Area(d) => d.original,
            Gesture::Panel(d) => d.original,
        }
    }
}
pub struct Editor {
    pub integration: Option<Rc<dyn crate::integration::Integration>>,
    pub layout: Layout,
    agent_build_slots: HashMap<String, (serde_json::Value, f32)>,
    pub tokens: Tokens,
    pub host: Option<Rc<RefCell<Host>>>,
    geometry: Geometry,
    dock_scroll: HashMap<String, f32>,
    dock_reported_heights: BTreeMap<String, f32>,
    dock_natural_heights: BTreeMap<String, f32>,
    dock_scroll_max: HashMap<String, f32>,
    last_scroll_focus: Option<FocusHandle>,
    area_hover: Option<(String, Option<crate::area::Corner>)>,
    reveal_panel: Option<String>,
    library_places: HashMap<String, DropTarget>,
    reveal_motion: Option<(String, f32, f32, std::time::Instant)>,
    header_wash: Option<(String, std::time::Instant)>,
    /// Panels sliding to a new place after a drop, a maximise or a
    /// workspace switch: each panel's offset from its new rect, springing to
    /// nothing (v1 moved panels the same way). Live resizes never slide.
    slides: PanelSlides,
    /// Tabs sliding along their strips after the same changes.
    tab_slides: crate::drag::TabSlides,
    /// The drag chip settling after a release.
    settle: Option<Settle>,
    /// Where each extension body was placed this frame (its panel's
    /// animated rect, less the header and the dividers' grab).
    native_body_rects: Vec<(String, Rect)>,
    maximized_panel: Option<String>,
    /// Tabs of panels whose owner went away for a moment (an extension
    /// reloading), and where they were: the panel returns there.
    /// Parked panels: where each goes back, and the tab that was showing in
    /// its stack if it wasn't that panel, which shows again when it returns.
    parked: HashMap<String, (DropTarget, Option<String>)>,
    /// The app's extension panels are still coming in (launch, a project
    /// opening): a tab whose panel has not registered yet keeps its place
    /// with a quiet body, not "Panel Unavailable" (`panels:waiting/ready`).
    awaiting_panels: bool,
    /// Panels the workspace names that nothing provides once the extensions
    /// are in (one from an extension not installed here): v1 keeps them in
    /// the workspace but gives them no slot, and they come back with their
    /// extension (`visibleDockPlan`). Parked meanwhile.
    absent: std::collections::BTreeSet<String>,
    /// Panels asked back before their extension had registered them (at
    /// launch): restored once the extensions are ready, never an error.
    pending_restores: Vec<String>,
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
    /// The project that choice was made for (`None`: Home). Another
    /// project bound to the window later never takes it: a new project
    /// starts with the default workspace as shipped (PM-454).
    preferred_for: Option<String>,
    /// Writes for the kernel, sent on the next frame.
    outbox: Vec<workspace::Write>,
    /// The window took another project and its store still describes the
    /// last one: the record shown belongs to that one, so it is never
    /// written as this project's workspace (PM-454).
    binding: bool,
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
            agent_build_slots: HashMap::new(),
            integration: None,
            layout,
            tokens,
            host,
            geometry: Geometry::default(),
            dock_scroll: HashMap::new(),
            dock_reported_heights: BTreeMap::new(),
            dock_natural_heights: BTreeMap::new(),
            dock_scroll_max: HashMap::new(),
            last_scroll_focus: None,
            area_hover: None,
            reveal_panel: None,
            library_places: HashMap::new(),
            reveal_motion: None,
            header_wash: None,
            slides: PanelSlides::default(),
            tab_slides: Default::default(),
            settle: None,
            native_body_rects: Vec::new(),
            maximized_panel: None,
            parked: HashMap::new(),
            awaiting_panels: true,
            absent: Default::default(),
            pending_restores: Vec::new(),
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
            preferred_for: None,
            outbox: vec![],
            binding: false,
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
    fn moving_name(
        &self,
        layout: &Layout,
        id: &str,
        stack: bool,
        panels: &layout::Panels,
    ) -> String {
        match layout.find(id) {
            Some((d, g, _)) if stack => layout.docks[d].groups[g]
                .tabs
                .iter()
                .map(|t| panels.title(t).to_string())
                .collect::<Vec<_>>()
                .join(" + "),
            _ => panels.title(id).to_string(),
        }
    }
    /// The panel or dock a drop is named after, in this layout.
    fn near_name(
        &self,
        near: layout::Near,
        drag: &PanelDrag,
        panels: &layout::Panels,
    ) -> (String, &'static str) {
        // Named after the tab that will show there once the dragged one left.
        let title = |d: usize, g: usize| {
            drag.original
                .shown_after(d, g, &drag.id, drag.stack)
                .map_or_else(String::new, |id| panels.title(&id).to_string())
        };
        match near {
            layout::Near::Tab(d, g) => (title(d, g), "tab"),
            layout::Near::Above(d, g) => (title(d, g), "above"),
            layout::Near::Below(d, g) => (title(d, g), "below"),
            layout::Near::Dock(d) => (
                dock_label(drag.original.docks.get(d).map_or("", |d| d.id.as_str())),
                "dock",
            ),
            layout::Near::Edge(e) => (dock_label(e.dock_id()), "dock"),
            layout::Near::Nothing => (String::new(), "dock"),
        }
    }
    /// A drag in progress put back as it began: Escape, or the window
    /// losing focus (the original app cancels then).
    pub fn cancel_gesture(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.gesture.take() else {
            return;
        };
        let moved = matches!(&g, Gesture::Panel(d) if d.moved);
        if let Gesture::Panel(drag) = &g
            && drag.moved
        {
            self.put_back(drag, cx);
        }
        self.slides.follow = false;
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
        self.preferred_for = Some(state.project.clone()).filter(|p| !p.is_empty());
        self.show_record(self.workspaces.active(self.preferred.as_deref()));
    }
    pub fn follow_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::env::var_os("PM_NATIVE_APPEARANCE").is_none() {
            let tokens = crate::theme::tokens(
                match window.appearance() {
                    WindowAppearance::Dark | WindowAppearance::VibrantDark => {
                        pm_design::Scheme::Dark
                    }
                    _ => pm_design::Scheme::Light,
                },
                crate::native::settings(),
            );
            // A window's appearance is reported more often than it changes:
            // the same look rebuilds nothing (PM-400).
            if tokens == self.tokens {
                return;
            }
            self.tokens = tokens;
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
        self.parked.clear();
        self.project = project;
        // The project's own workspace arrives with the store; until then the
        // window keeps showing what it showed, and writes none of it: the
        // kernel files a window's workspace under the project it holds when
        // the write is sent, and a new project (no workspace yet) took the
        // edited layout a new tab showed before it (PM-454).
        self.seen = None;
        self.binding = true;
        self.pending.clear();
        self.outbox
            .retain(|w| !matches!(w, workspace::Write::Active { .. }));
        self.persist_session();
    }
    fn dock_wheel(&mut self, event: &ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        if event.modifiers.control || event.modifiers.platform || event.modifiers.shift {
            return;
        }
        let by = -f32::from(
            window
                .scroll_delta_remaining(event.delta.pixel_delta(px(16.)))
                .y,
        );
        if by == 0. || !by.is_finite() {
            return;
        }
        let Some(di) = self.geometry.wheel_dock(
            f32::from(event.position.x),
            f32::from(event.position.y),
            self.tokens.metrics.splitter_reach,
        ) else {
            return;
        };
        let id = self.layout.docks[di].id.clone();
        self.reveal_panel = None;
        self.reveal_motion = None;
        let moved = self.scroll_dock(&id, by);
        // The gutter belongs to this column, even at its limit. Never pass a
        // wheel to the viewer beneath the divider's overlapping hit area.
        cx.stop_propagation();
        if moved {
            cx.notify();
        }
    }

    pub fn show_library_panel(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        if !self.panels(cx).contains(id) {
            return Err("This panel is no longer available.".into());
        }
        let closed = self.layout.find(id).is_none();
        self.shell_command(&format!("panel:activate:{id}"), cx);
        if closed && let Some(place) = self.library_places.get(id).cloned() {
            let _ = self.layout.move_panel_to(id, place);
        }
        if let Some((di, gi, ti)) = self.layout.find(id) {
            // Restoring its last slot may put it in a different folded stack.
            self.layout.docks[di].collapsed = false;
            self.layout.docks[di].groups[gi].collapsed = false;
            self.layout.docks[di].groups[gi].active = ti;
            let dock = self.layout.docks[di].id.clone();
            self.maximized_panel = None;
            self.save();
            Ok(dock)
        } else {
            Err(self.status.clone())
        }
    }

    /// Move a side column without saving transient viewport state (Electron's rule).
    pub fn scroll_dock(&mut self, id: &str, by: f32) -> bool {
        if !by.is_finite() {
            return false;
        }
        let old = self.dock_scroll.get(id).copied().unwrap_or(0.);
        let maximum = self.dock_scroll_max.get(id).copied().unwrap_or(0.);
        let next = (old + by).clamp(0., maximum);
        if next == old {
            return false;
        }
        self.dock_scroll.insert(id.into(), next);
        true
    }

    fn dock_body_fade(&self, dock: usize, body: Rect) -> (f32, f32) {
        let Some(viewport) = self.geometry.side_docks.get(&dock) else {
            return (0., 0.);
        };
        let id = &self.layout.docks[dock].id;
        let (top, bottom) = pm_design::dock_scroll_edges(
            self.dock_scroll.get(id).copied().unwrap_or(0.),
            self.dock_scroll_max.get(id).copied().unwrap_or(0.),
        );
        let depth = pm_design::DOCK_SCROLL_FADE;
        let top = top && body.y <= viewport.y + self.tokens.metrics.header;
        let bottom = bottom && body.y + body.h >= viewport.y + viewport.h;
        (if top { depth } else { 0. }, if bottom { depth } else { 0. })
    }

    /// Save the layout in its workspace record and session.
    pub fn save(&mut self) {
        // A parked panel keeps its place in the workspace (v1 kept a retired
        // panel's spec); only this window's view leaves it out.
        let mut written = self.layout.clone();
        for id in self.agent_build_slots.keys() {
            layout::remove_build(&mut written, id);
        }
        // Extension placeholders stay in the actual tree. Only the Agent
        // popover is removed from this window while retaining a docked home.
        for (id, (place, showing)) in self.parked.iter().filter(|(id, _)| id.as_str() == "agent") {
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
        let after = workspace::persistent_record(after);
        if self.binding {
            self.record = after;
            return;
        }
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
    /// This window's choice, when it was made for the project it shows now.
    fn preferred_here(&self) -> Option<&str> {
        self.preferred
            .as_deref()
            .filter(|_| self.preferred_for == self.workspaces.project_id)
    }
    fn show_record(&mut self, record: serde_json::Value) {
        self.maximized_panel = None;
        // Places in the layout this replaces; the new one's absent panels
        // are found again on the next frame.
        for id in std::mem::take(&mut self.absent) {
            self.parked.remove(&id);
        }
        self.layout = workspace::layout(&record);
        self.parked.retain(|id, _| self.layout.find(id).is_some());
        self.record = record;
    }
    /// Shows workspace `record` in this window and makes it the active one.
    fn activate_record(&mut self, record: serde_json::Value) {
        self.preferred = workspace::record_id(&record).map(str::to_owned);
        self.preferred_for = self.workspaces.project_id.clone();
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
        if self.binding && self.workspaces.project_id == integration.project_id(cx) {
            self.binding = false;
        }
        let target = self.workspaces.active(self.preferred_here());
        // Freeze the inherited workspace even if no layout edit follows.
        // Otherwise saving an old/new project writes a null workspace and
        // opening it later inherits whichever global layout is current then.
        if self.workspaces.project_id.is_some()
            && !self.workspaces.project.as_ref().is_some_and(workspace::usable)
        {
            if self.pending.is_empty() {
                self.show_record(target.clone());
                self.commit(target);
                self.persist_session();
            }
            // A stale null publication cannot acknowledge our pending write.
            return;
        }
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
        // One entry per name: the one this window shows, else its project's,
        // else the last used (PM-449).
        let prefer: Vec<&str> = [
            workspace::record_id(&self.record),
            self.workspaces
                .project
                .as_ref()
                .and_then(workspace::record_id),
            self.workspaces.current.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let offered = workspace::one_per_name(self.workspaces.offered(), &prefer);
        let names: Vec<String> = offered
            .iter()
            .map(|r| workspace::record_name(r).to_owned())
            .collect();
        offered
            .into_iter()
            .zip(workspace::told_apart(&names))
            .map(|(r, name)| {
                (
                    workspace::record_id(r).unwrap_or_default().to_owned(),
                    name,
                    workspace::is_builtin(r),
                )
            })
            .collect()
    }
    /// The names of the workspaces there are, as saved.
    fn workspace_names(&self) -> Vec<String> {
        self.workspaces
            .offered()
            .into_iter()
            .map(|r| workspace::record_name(r).to_owned())
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
            tab_index: host.tab_index(),
            pages: host.page_ids(),
            fullscreen: host.is_fullscreen(),
            name: (!self.project.is_empty()).then(|| host.shown_project_name()).flatten(),
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
        let key =
            !crate::session::quitting() && self.host.as_ref().is_some_and(|h| h.borrow().is_key());
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
                // A group moves as one: its other tabs take the new frame.
                if let Some(h) = &v.host {
                    h.borrow().persist_other_tabs();
                }
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
        let mut remembered = self
            .integration
            .as_ref()
            .map(|i| i.remembered_panel_titles(cx))
            .unwrap_or_default();
        // A panel waiting for its extension is named, never shown by its id
        // ("background-remover.panel", PM-454).
        for id in self.layout.open_panels() {
            if !layout::PANELS.contains(&id)
                && !contributed.iter().chain(&remembered).any(|(p, _)| p == id)
            {
                remembered.push((id.to_owned(), layout::readable_title(id)));
            }
        }
        layout::Panels::new(contributed).with_remembered(remembered).while_loading(
            self.integration.as_ref().is_some_and(|i| i.panels_loading(cx)),
        )
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
            transition_refusal: self
                .integration
                .as_ref()
                .and_then(|i| i.transition_refusal(cx)),
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
    /// Once the extensions are in, a panel nothing provides takes no slot
    /// (its stack's other tabs and the column fill the space) and keeps its
    /// place in the workspace; it comes back when something registers it.
    fn settle_absent_panels(&mut self, cx: &mut Context<Self>) {
        if self.awaiting_panels || self.integration.is_none() {
            return;
        }
        let panels = self.panels(cx);
        let back: Vec<String> = self
            .absent
            .iter()
            .filter(|id| panels.contains(id))
            .cloned()
            .collect();
        for id in back {
            self.absent.remove(&id);
            self.shell_command(&format!("panel:restore:{id}"), cx);
        }
        let gone: Vec<String> = self
            .layout
            .open_panels()
            .into_iter()
            .filter(|id| !panels.contains(id) && !self.agent_build_slots.contains_key(*id))
            .map(str::to_owned)
            .collect();
        for id in gone {
            self.shell_command(&format!("panel:park:{id}"), cx);
            self.absent.insert(id);
        }
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
        if kind == "panels" && matches!(verb, "waiting" | "ready") {
            self.awaiting_panels = verb == "waiting";
            if verb == "ready" {
                let known = self.panels(cx);
                for id in std::mem::take(&mut self.pending_restores) {
                    // One whose extension never came stays away quietly.
                    if known.contains(&id) {
                        self.shell_command(&format!("panel:restore:{id}"), cx);
                    }
                }
                self.settle_absent_panels(cx);
            }
            cx.notify();
            return true;
        }
        let panels = self.panels(cx);
        if kind == "panel"
            && (verb == "close" || (verb == "toggle" && self.layout.find(rest).is_some()))
            && self.agent_build_slots.contains_key(rest)
        {
            self.release_agent_panel(rest);
            if let Some(integration) = self.integration.clone() {
                integration.panel_closed(rest, cx);
            }
            self.save();
            cx.notify();
            return true;
        }
        if kind == "panel"
            && matches!(verb, "close" | "toggle")
            && let Some((dock, group, index)) = self.layout.find(rest)
        {
            let place = if self.layout.docks[dock].groups[group].tabs.len() == 1 {
                DropTarget::Split { dock, group }
            } else {
                DropTarget::Tab { dock, group, index }
            };
            self.library_places.insert(rest.into(), place);
        }
        let result = match (kind, verb) {
            ("panel", "toggle") => {
                if self.layout.open_panels().contains(&rest) && layout::closable(rest) {
                    self.layout.close_panel(rest)
                } else {
                    self.layout.show_panel(rest, &panels)
                }
            }
            ("panel", "activate") => {
                let result = self.layout.show_panel(rest, &panels);
                if let Err(error) = result {
                    self.status = error.to_string();
                } else {
                    if let Some((place, _)) = self.parked.remove(rest) {
                        let _ = self.layout.move_panel_to(rest, place);
                    }
                    if let Some((di, gi, ti)) = self.layout.find(rest) {
                        self.layout.docks[di].collapsed = false;
                        self.layout.docks[di].groups[gi].collapsed = false;
                        self.layout.docks[di].groups[gi].active = ti;
                    }
                    self.reveal_motion = None;
                    self.reveal_panel = Some(rest.into());
                    self.header_wash = (!self.tokens.accessibility.reduce_motion)
                        .then(|| (rest.into(), std::time::Instant::now()));
                    self.save();
                }
                cx.notify();
                return true;
            }
            ("panel", "registered") => {
                // A reconnect refreshes saved placeholders. It must not reopen
                // something closed meanwhile, or add it to a different project.
                if rest != "agent" { self.parked.remove(rest); }
                cx.notify();
                return true;
            }
            ("panel", "show") => match self.parked.remove(rest) {
                // Registration completes in place; it must not select or move
                // the saved placeholder when extensions arrive out of order.
                Some(_) if self.layout.find(rest).is_some() => Ok(()),
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
            // A panel coming back (its extension restarted as the project
            // opens): in its place, or shown if it had none, but never taking
            // the showing tab of a stack it is already in (PM-427).
            ("panel", "restore") => match self.parked.remove(rest) {
                Some((place, _)) if self.layout.find(rest).is_none() && panels.contains(rest) => {
                    self.layout.insert_at(rest, place);
                    Ok(())
                }
                _ if self.layout.find(rest).is_some() => Ok(()),
                // Its extension has not registered it yet (launch): it comes
                // back when the extensions are ready, not as "Unknown panel".
                _ if !panels.contains(rest) => {
                    if !self.pending_restores.iter().any(|id| id == rest) {
                        self.pending_restores.push(rest.to_owned());
                    }
                    Ok(())
                }
                _ => self.layout.restore_panel(rest, &panels),
            },
            ("panel", "background-close") => {
                if let Some((dock, group, tab)) = self.layout.find(rest)
                    && self.layout.docks[dock].groups[group].active != tab
                {
                    return self.shell_command(&format!("panel:close:{rest}"), cx);
                }
                Ok(())
            }
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
                    if rest == "agent" {
                        self.layout.close_panel(rest)
                    } else {
                        // Keep the entire group, size, order and active tab.
                        // The unavailable view is replaced when registration finishes.
                        Ok(())
                    }
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
            ("area", "split") => {
                let mut parts = rest.split(':');
                let source = parts.next().unwrap_or("");
                let new = parts.next().unwrap_or("");
                let top = parts.next() == Some("top");
                let height = parts
                    .next()
                    .and_then(|s| s.parse::<f32>().ok())
                    .unwrap_or(0.);
                let total = self
                    .layout
                    .find(source)
                    .and_then(|(d, g, _)| {
                        self.geometry
                            .panels
                            .iter()
                            .find(|p| (p.dock, p.group) == (d, g))
                    })
                    .map_or(0., |p| p.rect.h);
                if panels.ids().contains(&new) {
                    self.layout
                        .split_area(source, new, top, height, total, self.tokens.metrics.gap)
                } else {
                    Err(anyhow::anyhow!("Unknown panel"))
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
                    h.borrow().prompt_workspace_name(&workspace::save_name(
                        workspace::record_name(&self.record),
                        &self.workspace_names(),
                    ));
                }
                Ok(())
            }
            ("layout", "save") => layout::layout_name(rest).map(|name| {
                // Never a second workspace of a name already there.
                let name = workspace::free_name(&name, &self.workspace_names());
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
    /// Pointer focus has no panel ring. Keep AppKit and GPUI in agreement so
    /// the previous field, toolbar control or splitter cannot retain arrows.
    pub fn focus_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(self.panel_focus.get("viewer").unwrap_or(&self.focus), cx);
        self.ringed_panel = None;
        self.split_focus = None;
        if let Some(host) = &self.host {
            host.borrow_mut().focus_canvas();
        }
        cx.notify();
    }

    fn note_transition_refusal(&self, command: &str, cx: &mut App) {
        if command == "addDefaultTransition"
            && let Some(integration) = &self.integration
            && let Some(reason) = integration.transition_refusal(cx)
        {
            integration.note(&reason, cx);
        }
    }
    pub fn command(&mut self, cmd: Command, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = cmd.name.strip_prefix("dockContent:") {
            if self.layout.find(id).is_some()
                && let Some(height) = cmd
                    .args
                    .first()
                    .and_then(|v| v.as_f64())
                    .filter(|h| h.is_finite() && *h > 0.)
            {
                let height = height as f32 + self.tokens.metrics.header;
                if self
                    .dock_reported_heights
                    .get(id)
                    .is_none_or(|old| (*old - height).abs() > 1.)
                {
                    self.dock_reported_heights.insert(id.into(), height);
                    cx.notify();
                }
            }
            return;
        }
        if let Some(id) = cmd.name.strip_prefix("dockScroll:") {
            if let Some((di, _, _)) = self.layout.find(id) {
                let dock = self.layout.docks[di].id.clone();
                let by = cmd.args.first().and_then(|v| v.as_f64()).unwrap_or(0.) as f32;
                if by.is_finite() && self.scroll_dock(&dock, by) {
                    cx.notify();
                }
            }
            return;
        }
        if let Some(id) = cmd.name.strip_prefix("dockReveal:") {
            if let Some(p) = self
                .geometry
                .panels
                .iter()
                .find(|p| {
                    self.layout.docks[p.dock].groups[p.group]
                        .tabs
                        .iter()
                        .any(|tab| tab == id)
                })
                .cloned()
                && let Some(bounds) = self.geometry.side_docks.get(&p.dock).copied()
            {
                let relative = cmd.args.first().and_then(|v| v.as_f64()).unwrap_or(0.) as f32;
                let height = cmd.args.get(1).and_then(|v| v.as_f64()).unwrap_or(24.) as f32;
                let y = p.rect.y + self.tokens.metrics.header + relative;
                let by = if y < bounds.y {
                    y - bounds.y
                } else {
                    (y + height.min(bounds.h) - bounds.y - bounds.h).max(0.)
                };
                let dock = self.layout.docks[p.dock].id.clone();
                if by.is_finite() && self.scroll_dock(&dock, by) {
                    cx.notify();
                }
            }
            return;
        }

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
            self.note_transition_refusal(&cmd.name, cx);
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
            // Tabs mode: a new tab of this window; else a new window.
            "newTab" => crate::open_editor_tab(cx, self.window_id),
            // The window's tab group changed (a tab joined): the session
            // records it.
            "persistSession" => self.persist_session(),
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
        if !matches!(&self.gesture, Some(Gesture::Panel(d)) if d.live && d.id == id) {
            if let Some(g) = self.gesture.take() {
                self.layout = g.original();
            }
            self.gesture = Some(Gesture::Panel(self.new_drag(id, at, false, true)));
        }
        if let Some(Gesture::Panel(drag)) = &mut self.gesture {
            drag.at = at;
            drag.moved = true;
        }
        self.retarget(true);
    }
    /// A drag of `id` (its whole stack when `stack`) beginning at `at`, with
    /// the layout, the geometry and the column scroll as they are now.
    fn new_drag(&self, id: &str, at: Point<Pixels>, stack: bool, live: bool) -> PanelDrag {
        PanelDrag {
            id: id.into(),
            stack,
            start: at,
            original: self.layout.clone(),
            at,
            moved: false,
            preview: None,
            shown: None,
            rest: ((f32::from(at.x), f32::from(at.y)), crate::drag::now()),
            live,
            geometry: self.geometry.clone(),
            scroll: self.dock_scroll.clone(),
            scrolled: HashMap::new(),
            lifted: None,
            scroll_tick: None,
        }
    }
    /// The zones a drag's targets are read from: the panels as drawn at
    /// pick-up, moved only by the drag's own edge scroll. The layout shown
    /// re-flows to each preview; the zones never move under the pointer.
    fn drag_zones(drag: &PanelDrag) -> Geometry {
        let mut zones = drag.geometry.clone();
        for p in &mut zones.panels {
            if let Some(by) = drag
                .original
                .docks
                .get(p.dock)
                .and_then(|d| drag.scrolled.get(&d.id))
            {
                p.rect.y -= by;
            }
        }
        zones
    }
    /// Chooses the drag's target for where the pointer is (steadily: see
    /// `drag::steady`) and, once the pointer rests on it (or at once when
    /// `now`, or for a tab moving within its own strip), shows the layout a
    /// drop there would give: the layout the drag began in with that drop
    /// made, or that layout itself over no target. Panels and tabs travel
    /// to it on the move spring.
    fn retarget(&mut self, now: bool) {
        let Some(Gesture::Panel(drag)) = &mut self.gesture else {
            return;
        };
        if !drag.live || !drag.moved {
            return;
        }
        let time = crate::drag::now();
        if drag.lifted.is_none() {
            // The press itself may have re-laid the panels (the tab it
            // showed): targets are read from the panels as drawn at lift.
            drag.lifted = Some(time);
            drag.geometry = self.geometry.clone();
            drag.scroll = self.dock_scroll.clone();
        }
        let zones = Self::drag_zones(drag);
        let (bounds, tokens) = (self.dock_bounds, &self.tokens);
        let at = (f32::from(drag.at.x), f32::from(drag.at.y));
        if !crate::drag::within_rest(drag.rest.0, at) {
            drag.rest = (at, time);
        }
        let preview = crate::drag::steady(drag.preview, at, |at| {
            drag.original
                .drop_target_for(&zones, bounds, at, tokens, &drag.id, drag.stack)
        });
        let mut scratch = drag.original.clone();
        drag.preview = preview.filter(|p| scratch.apply_drop(&drag.id, p, drag.stack).is_ok());
        let target = |p: &Option<DropPreview>| p.map(|p| p.target);
        // A tab moving along its own strip follows the pointer, as Safari's.
        let own_strip = match (target(&drag.preview), drag.original.find(&drag.id)) {
            (Some(DropTarget::Tab { dock, group, .. }), Some((d, g, _))) => {
                !drag.stack && (dock, group) == (d, g)
            }
            _ => false,
        };
        if target(&drag.preview) != target(&drag.shown)
            && (now
                || own_strip
                || tokens.accessibility.reduce_motion
                || crate::drag::rested(drag.rest.1, time))
        {
            drag.shown = drag.preview;
        }
        let mut shown = drag.original.clone();
        if let Some(p) = drag.shown
            && shown.apply_drop(&drag.id, &p, drag.stack).is_err()
        {
            shown = drag.original.clone();
            drag.shown = None;
        }
        // In the hand over no target: its stack already shows the tab that
        // shows once it has left (a release here still puts it back).
        if drag.shown.is_none()
            && !drag.stack
            && let Some((d, g, _)) = shown.find(&drag.id)
            && let Some(next) = shown.shown_after(d, g, &drag.id, false)
            && let Some(tab) = shown.docks[d].groups[g].tabs.iter().position(|t| *t == next)
        {
            shown.docks[d].groups[g].active = tab;
        }
        self.slides.follow = true;
        if shown != self.layout {
            self.layout = shown;
            self.slides.armed = true;
            self.tab_slides.armed = true;
        }
    }
    /// Each frame of a drag: the layout catches up with the target once
    /// the pointer rests, and a scrolling column scrolls under a drag held
    /// near its edge.
    fn drag_frame(&mut self, window: &mut Window) {
        let waiting = |v: &Self| {
            matches!(&v.gesture, Some(Gesture::Panel(d)) if d.live && d.moved
                && d.preview.map(|p| p.target) != d.shown.map(|p| p.target))
        };
        if waiting(self) {
            self.retarget(false);
            if waiting(self) {
                window.request_animation_frame();
            }
        }
        self.drag_edge_scroll(window);
    }
    /// While a drag is held near the top or bottom of a scrolling column,
    /// the column scrolls towards that edge and the target follows.
    fn drag_edge_scroll(&mut self, window: &mut Window) {
        let Some(Gesture::Panel(drag)) = &mut self.gesture else {
            return;
        };
        if !drag.live || !drag.moved {
            return;
        }
        let (x, y) = (f32::from(drag.at.x), f32::from(drag.at.y));
        let column = self
            .geometry
            .side_docks
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .and_then(|(di, r)| {
                let id = self.layout.docks.get(*di)?.id.clone();
                let speed = crate::drag::autoscroll(y, r.y, r.y + r.h);
                let old = self.dock_scroll.get(&id).copied().unwrap_or(0.);
                let most = self.dock_scroll_max.get(&id).copied().unwrap_or(0.);
                ((speed < 0. && old > 0.) || (speed > 0. && old < most))
                    .then_some((id, speed, old, most))
            });
        let Some((id, speed, old, most)) = column else {
            drag.scroll_tick = None;
            return;
        };
        let now = crate::drag::now();
        let dt = drag.scroll_tick.replace(now).map_or(0., |last| {
            now.saturating_duration_since(last).as_secs_f32().min(0.05)
        });
        window.request_animation_frame();
        let next = (old + speed * dt).clamp(0., most);
        if next == old {
            return;
        }
        *drag.scrolled.entry(id.clone()).or_default() += next - old;
        self.dock_scroll.insert(id, next);
        self.retarget(false);
    }
    /// A drag let go over no target, or cancelled: the columns scroll back
    /// to where they were, the panels travel home, and the chip glides back
    /// to where it was picked up.
    fn put_back(&mut self, drag: &PanelDrag, cx: &App) {
        for (id, at) in &drag.scroll {
            self.dock_scroll.insert(id.clone(), *at);
        }
        self.slides.armed = true;
        self.tab_slides.armed = true;
        if !drag.live || self.tokens.accessibility.reduce_motion {
            return;
        }
        let panels = self.panels(cx);
        let o = crate::drag::CHIP_OFFSET;
        self.settle = Some(Settle {
            name: self.moving_name(&drag.original, &drag.id, drag.stack, &panels),
            from: (f32::from(drag.at.x) + o, f32::from(drag.at.y) + o),
            to: (f32::from(drag.start.x), f32::from(drag.start.y)),
            slot: None,
            since: crate::drag::now(),
        });
    }
    /// Where tab `id` (or a lone title) is drawn in its header, as last
    /// rendered. For headless recordings and tests.
    #[doc(hidden)]
    pub fn tab_rect(&self, id: &str) -> Option<Rect> {
        let (dock, group, tab) = self.layout.find(id)?;
        let p = self
            .geometry
            .panels
            .iter()
            .find(|p| (p.dock, p.group) == (dock, group))?;
        let header = self.geometry.header_rect(p, &self.tokens);
        let (_, left, right) = self
            .geometry
            .tabs
            .get(&(dock, group))?
            .iter()
            .find(|(i, ..)| *i == tab)?;
        Some(Rect {
            x: *left,
            y: header.y,
            w: right - left,
            h: header.h,
        })
    }
    /// Whether a panel drag is in progress (past its threshold).
    #[doc(hidden)]
    pub fn dragging_panel(&self) -> bool {
        matches!(&self.gesture, Some(Gesture::Panel(d)) if d.moved)
    }
    /// The slot a dragged panel shows in: its place in the layout shown,
    /// less what a scrolling column hides of it.
    fn slot_rect(&self, id: &str) -> Option<Rect> {
        let (dock, group, _) = self.layout.find(id)?;
        let rect = self
            .geometry
            .panels
            .iter()
            .find(|p| (p.dock, p.group) == (dock, group))?
            .rect;
        let rect = self
            .geometry
            .side_docks
            .get(&dock)
            .map_or(rect, |column| rect.intersection(*column));
        (rect.w > 0. && rect.h > 0.).then_some(rect)
    }
    /// Where the chip settles after a drop: the panel's tab, or its title.
    fn landing_point(&self, id: &str) -> Option<(f32, f32)> {
        let m = self.tokens.metrics;
        let (d, g, ti) = self.layout.find(id)?;
        let r = self
            .geometry
            .panels
            .iter()
            .find(|p| (p.dock, p.group) == (d, g))?
            .rect;
        let x = self
            .geometry
            .tabs
            .get(&(d, g))
            .and_then(|spans| spans.iter().find(|(i, ..)| *i == ti))
            .map_or(r.x + m.inset, |(_, left, _)| *left);
        Some((x, r.y + (m.header - m.hit) / 2.))
    }
    /// A panel that is not in the workspace (the Agent popover, a window of
    /// its own) dragged over it to `at` (this window's points): the same drop
    /// preview a tab drag shows, for the panel as Pin would first place it.
    /// True when a drop there would dock it.
    pub fn drag_from_outside(&mut self, id: &str, at: (f32, f32), cx: &mut Context<Self>) -> bool {
        let preview = self.outside_target(id, at, cx);
        let pointer = point(px(at.0), px(at.1));
        let mut drag = match self.gesture.take() {
            Some(Gesture::Panel(d)) if !d.live => d,
            other => {
                if let Some(g) = other {
                    self.layout = g.original();
                }
                self.new_drag(id, pointer, false, false)
            }
        };
        drag.id = id.into();
        drag.at = pointer;
        drag.moved = true;
        drag.lifted.get_or_insert_with(crate::drag::now);
        drag.preview = preview;
        self.gesture = Some(Gesture::Panel(drag));
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
            .and_then(|_| next.move_panel_to(id, preview.target));
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
        // The window shows the drop already: make it from the layout the
        // drag began in, as a release does.
        self.slides.follow = false;
        self.layout = drag.original.clone();
        let preview = drag.preview?;
        self.layout
            .apply_drop(&drag.id, &preview, drag.stack)
            .ok()?;
        self.save();
        Some(preview.target)
    }
    /// Ends the drag in progress as a release does: a panel moved lands
    /// where its preview showed, a resize keeps the sizes it reached.
    fn end_gesture(&mut self, cx: &mut Context<Self>) {
        let Some(g) = self.gesture.take() else {
            return;
        };
        self.resize_live = false;
        if let Gesture::Area(drag) = &g {
            if drag.moved {
                if let crate::area::Action::Split { top, height } = drag.action {
                    let panels = self.panels(cx);
                    let mut ids: Vec<_> = panels
                        .ids()
                        .into_iter()
                        .filter(|id| *id != "toolbar" && self.layout.find(id).is_none())
                        .collect();
                    ids.sort_by_key(|id| panels.title(id).to_lowercase());
                    let items: Vec<_> = ids
                        .into_iter()
                        .map(|id| {
                            MenuEntry::item(
                                panels.title(id),
                                format!(
                                    "area:split:{}:{id}:{}:{height}",
                                    drag.id,
                                    if top { "top" } else { "bottom" }
                                ),
                            )
                        })
                        .collect();
                    if let Some(host) = &self.host {
                        host.borrow().context_menu(
                            &items,
                            f32::from(drag.at.x),
                            f32::from(drag.at.y),
                        );
                    }
                    if items.is_empty() {
                        self.notify_status("Every panel is already open".into(), cx);
                    }
                } else if let Err(err) = self.layout.apply_area(
                    &drag.id,
                    &drag.action,
                    &self.geometry,
                    self.tokens.metrics.gap,
                ) {
                    self.notify_status(err.to_string(), cx);
                }
            }
        }
        self.slides.follow = false;
        if let Gesture::Panel(drag) = g
            && drag.moved
        {
            // The original app says what happened: where it went, that it
            // was already there, or that it was no drop zone.
            let panels = self.panels(cx);
            let name = self.moving_name(&drag.original, &drag.id, drag.stack, &panels);
            // The window shows the drop already (`retarget`); the release
            // makes it from the layout the drag began in, as it always has:
            // one change, saved once.
            if drag.live {
                self.layout = drag.original.clone();
            }
            if drag.preview.is_none() {
                self.put_back(&drag, cx);
            }
            let message = match drag.preview {
                Some(p) => {
                    let (near, kind) = self.near_name(p.near, &drag, &panels);
                    let moved = self.layout.apply_drop(&drag.id, &p, drag.stack);
                    if moved.is_ok() && !self.tokens.accessibility.reduce_motion {
                        // The chip settles into the panel's new place.
                        let o = crate::drag::CHIP_OFFSET;
                        let from = (f32::from(drag.at.x) + o, f32::from(drag.at.y) + o);
                        self.settle = Some(Settle {
                            name: name.clone(),
                            from,
                            // (Released before the layout caught up: it fades
                            // where it is while the panels travel.)
                            to: self
                                .landing_point(&drag.id)
                                .filter(|_| drag.shown.map(|s| s.target) == Some(p.target))
                                .unwrap_or(from),
                            slot: (!matches!(p.target, DropTarget::Tab { .. }))
                                .then(|| drag.id.clone()),
                            since: crate::drag::now(),
                        });
                    }
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
                    &Self::drag_zones(&drag),
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
        self.tab_slides.armed = true;
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
        self.settle = None;
        self.gesture = Some(Gesture::Panel(self.new_drag(id, at, stack, true)));
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
                    self.slides.follow = false;
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
    /// The header's window-space rect after column scrolling.
    pub fn panel_header_rect(&self, id: &str) -> Option<Rect> {
        let (dock, group, _) = self.layout.find(id)?;
        self.geometry
            .panels
            .iter()
            .find(|p| (p.dock, p.group) == (dock, group))
            .map(|p| self.geometry.header_rect(p, &self.tokens))
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
    pub fn reserve_agent_panel(
        &mut self,
        id: &str,
        placement: &serde_json::Value,
        height: f32,
    ) -> serde_json::Value {
        self.slides.armed = true;
        self.maximized_panel = None;
        let used = layout::reserve_build(&mut self.layout, id, placement, height);
        self.agent_build_slots
            .insert(id.into(), (used.clone(), height));
        used
    }
    pub fn release_agent_panel(&mut self, id: &str) {
        if self.layout.find(id).is_some() {
            self.slides.armed = true;
        }
        self.agent_build_slots.remove(id);
        layout::remove_build(&mut self.layout, id);
    }
    /// Persist the final id in exactly the pre-reserved build geometry.
    pub fn commit_agent_panel_layout(&mut self, cx: &mut Context<Self>) {
        self.save();
        cx.notify();
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
            flex_bases: self.layout.flex_bases.clone(),
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
/// move spring (`drag::move_spring`: no overshoot; Reduce Motion places it); anything else (a splitter drag, a window resize) places
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
    /// A panel drag is in progress: a live change under it (its edge
    /// scroll) carries the slides under way along instead of ending them.
    pub follow: bool,
}
impl PanelSlides {
    /// Takes this frame's rects; true while something still travels.
    pub fn place(&mut self, placed: &[(String, Rect)], t: &Tokens) -> bool {
        let dt = self.clock.tick(crate::drag::now());
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
                let spring = crate::drag::move_spring(t);
                if spring.snaps() {
                    self.moving.remove(id);
                    continue;
                }
                slide.animate_to([0., 0.], spring);
                self.moving.insert(id.clone(), slide);
            }
        } else if !self.follow
            && placed
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
/// An extension body inside its panel's rect: below the header, short of
/// the dividers' grab (`edge`) on the other three sides.
fn native_body(panel: Rect, edge: f32, header: f32) -> Rect {
    Rect {
        x: panel.x + edge,
        y: panel.y + header,
        w: (panel.w - edge * 2.).max(0.),
        h: (panel.h - header - edge).max(0.),
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
    fn titlebar_band(top: f32, fill: u32) -> Div {
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(top))
            .bg(rgb(fill))
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
            h.native_panel_clips.borrow_mut().clear();
            h.native_panel_fades.borrow_mut().clear();
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
            .child(Self::titlebar_band(top, self.tokens.colors.window))
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
/// Whether a control (a button, field or list) has the keyboard, so Return
/// and the arrows are its own. The viewer's canvas is not a control: after a
/// click on a layer there, Return edits a text layer and the arrows nudge
/// (PM-480).
fn control_has_keyboard(focused_elsewhere: bool, in_viewer: bool) -> bool {
    focused_elsewhere && !in_viewer
}
#[cfg(test)]
mod control_focus_tests {
    #[test]
    fn the_viewer_canvas_is_not_a_control() {
        assert!(super::control_has_keyboard(true, false), "a panel's field or list");
        assert!(!super::control_has_keyboard(true, true), "a layer clicked in the viewer");
        assert!(!super::control_has_keyboard(false, false), "the editor itself");
    }
}
impl Render for Editor {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Development builds: a frame as slow as asked, to check that input
        // keeps its order behind it (PM-465).
        #[cfg(debug_assertions)]
        if let Some(ms) = std::env::var("PM_NATIVE_SLOW_FRAME_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
        {
            std::thread::sleep(std::time::Duration::from_millis(ms.min(2000)));
        }
        let panels = self.panels(cx);
        if let Some(id) = self.integration.as_ref().and_then(|i| i.project_id(cx))
            && self.project != id
        {
            self.bind_project(id);
        }
        self.sync_workspaces(cx);
        self.settle_absent_panels(cx);
        for (id, (placement, height)) in &self.agent_build_slots {
            if !self.layout.open_panels().contains(&id.as_str()) {
                layout::reserve_build(&mut self.layout, id, placement, *height);
            }
        }
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
        self.drag_frame(w);
        // Freeze content measurements during a splitter drag, so every delta
        // uses the same starting geometry. Explicitly resized heights win below.
        if !matches!(self.gesture, Some(Gesture::Resize(..))) {
            self.dock_natural_heights = self.dock_reported_heights.clone();
            for dock in &self.layout.docks {
                for group in &dock.groups {
                    let id = group.active();
                    if let Some(height) = self
                        .integration
                        .as_ref()
                        .and_then(|i| i.natural_panel_height(id, cx))
                    {
                        self.dock_natural_heights.insert(id.into(), height);
                    }
                }
            }
        }
        self.geometry = if self.maximized_panel.is_some() {
            self.layout.geometry_with_maximized(
                self.dock_bounds,
                &t,
                self.maximized_panel.as_deref(),
            )
        } else {
            self.layout.geometry_with_panel_heights(
                self.dock_bounds,
                &t,
                &self.dock_natural_heights,
            )
        };
        if let Some(id) = self.reveal_panel.take()
            && let Some((di, gi, _)) = self.layout.find(&id)
            && let Some(bounds) = self.geometry.side_docks.get(&di)
            && let Some(panel) = self
                .geometry
                .panels
                .iter()
                .find(|p| p.dock == di && p.group == gi)
        {
            let dock = self.layout.docks[di].id.clone();
            let old = self.dock_scroll.get(&dock).copied().unwrap_or(0.);
            let target = if panel.rect.y - old < bounds.y {
                panel.rect.y - bounds.y
            } else {
                old.max(panel.rect.y + panel.rect.h.min(bounds.h) - bounds.y - bounds.h)
            }
            .max(0.);
            if self.tokens.accessibility.reduce_motion {
                self.dock_scroll.insert(dock, target);
            } else if (target - old).abs() > 0.5 {
                self.reveal_motion = Some((dock, old, target, std::time::Instant::now()));
            }
        }
        if let Some((dock, from, to, start)) = self.reveal_motion.clone() {
            let progress = (start.elapsed().as_secs_f32() / 0.2).min(1.);
            let eased = 1. - (1. - progress).powi(3);
            self.dock_scroll.insert(dock, from + (to - from) * eased);
            if progress < 1. {
                w.request_animation_frame();
            } else {
                self.reveal_motion = None;
            }
        }
        // Keep input/drop geometry in window coordinates, including the column offset.
        for (&di, bounds) in &self.geometry.side_docks {
            let id = &self.layout.docks[di].id;
            let bottom = self
                .geometry
                .panels
                .iter()
                .filter(|p| p.dock == di)
                .map(|p| p.rect.y + p.rect.h)
                .fold(bounds.y, f32::max);
            let offset = self.dock_scroll.entry(id.clone()).or_default();
            let maximum = (bottom - bounds.y - bounds.h).max(0.);
            // Electron ignores a <=1 px range; fractional flex rounding
            // must not turn a fitting column into a scrollable one.
            let maximum = if maximum <= 1. { 0. } else { maximum };
            self.dock_scroll_max.insert(id.clone(), maximum);
            *offset = offset.clamp(0., maximum);
            for p in self.geometry.panels.iter_mut().filter(|p| p.dock == di) {
                p.rect.y -= *offset;
            }
            for d in self
                .geometry
                .dividers
                .iter_mut()
                .filter(|d| d.dock == di && d.group.is_some())
            {
                d.rect.y -= *offset;
            }
        }
        let focused = self
            .panel_focus
            .iter()
            .find(|(_, f)| f.contains_focused(w, cx))
            .map(|(id, _)| id.clone());
        let focused_handle = w.focused(cx);
        if focused_handle != self.last_scroll_focus {
            self.last_scroll_focus = focused_handle;
            if let Some(p) = self
                .geometry
                .panels
                .iter()
                .find(|p| {
                    Some(self.layout.docks[p.dock].groups[p.group].active()) == focused.as_deref()
                })
                .cloned()
                && let Some(bounds) = self.geometry.side_docks.get(&p.dock).copied()
                // Only a panel scrolled wholly out of the column (reached by
                // the keyboard) is brought into view. One that shows is where
                // the person is working: a press in it (focusing it) must not
                // move it, or the control under the pointer leaves before the
                // release (the Colour panel's Add row, scrolled past its tabs).
                && (p.rect.y + p.rect.h <= bounds.y || p.rect.y >= bounds.y + bounds.h)
            {
                let by = if p.rect.y < bounds.y {
                    p.rect.y - bounds.y
                } else {
                    (p.rect.y + p.rect.h.min(bounds.h) - bounds.y - bounds.h).max(0.)
                };
                let id = self.layout.docks[p.dock].id.clone();
                if self.scroll_dock(&id, by) {
                    cx.notify();
                }
            }
        }
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
            (a.rect.y
                + self
                    .dock_scroll
                    .get(&self.layout.docks[a.dock].id)
                    .copied()
                    .unwrap_or(0.))
            .total_cmp(
                &(b.rect.y
                    + self
                        .dock_scroll
                        .get(&self.layout.docks[b.dock].id)
                        .copied()
                        .unwrap_or(0.)),
            )
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
                // The first pane is top-left: it has the body's bottom-left
                // corner when it reaches the bottom, never the bottom-right.
                let bottom = (first.y + first.h - (r.y + r.h)).abs() < 0.5;
                h.borrow().viewer_corners.set([bottom, false]);
                *h.borrow().viewer_tiles.borrow_mut() = tiles;
                Some(first)
            }
            (Some(h), _) => {
                h.borrow().viewer_tiles.borrow_mut().clear();
                h.borrow().viewer_corners.set([true, true]);
                viewer
            }
            _ => viewer,
        };
        if let Some(h) = &self.host {
            let h = h.borrow();
            h.viewer_rect.set(viewer);
            // The canvas inside the viewer (its gizmos) has a focus handle of
            // its own: a click on a layer there leaves the keyboard with the
            // canvas, not with a control (PM-480).
            let in_viewer = self
                .panel_focus
                .get("viewer")
                .is_some_and(|p| p.contains_focused(w, cx));
            h.canvas_focus.set(
                viewer.is_some()
                    && (in_viewer || w.focused(cx).is_none_or(|f| f == self.focus)),
            );
            h.publish_menu_state(&self.menu_state(cx));
            h.keyboard_control_focus.set(control_has_keyboard(
                w.focused(cx).is_some_and(|f| {
                    f != self.focus && !self.panel_focus.values().any(|p| *p == f)
                }),
                in_viewer,
            ));
        }
        let native = self.host.is_some();
        let canvas_viewer = viewer;
        let viewer_corners = self
            .host
            .as_ref()
            .map_or([true, true], |h| h.borrow().viewer_corners.get());
        let (ground, ground_alpha) = t.ground();
        // The native layers in the holes round their bottom corners (the
        // viewer a border inside its panel, an extension body its inset).
        let viewer_radius = (m.panel_radius - m.border).max(0.);
        let body_radius = (m.panel_radius - m.border.max(m.splitter_reach)).max(0.);
        let (line, line_alpha) = t.line();
        // Extension pages sit below GPUI too; their bodies are read at paint
        // time, after this frame's layout has recorded them.
        let native_bodies = self.host.as_ref().map(|h| h.borrow().native_panels.clone());
        let dragging = matches!(&self.gesture, Some(Gesture::Panel(d)) if d.moved);
        // A resize keeps its cursor wherever the pointer goes until release.
        let resizing = match &self.gesture {
            Some(Gesture::Resize(d, ..)) if d.horizontal => Some(CursorStyle::ResizeUpDown),
            Some(Gesture::Resize(..)) => Some(CursorStyle::ResizeLeftRight),
            Some(Gesture::Area(..)) => Some(CursorStyle::Crosshair),
            _ => None,
        };
        let mut root = div()
            .id("editor")
            .on_scroll_wheel(
                cx.listener(|this, e: &ScrollWheelEvent, window, cx| {
                    this.dock_wheel(e, window, cx)
                }),
            )
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
                        // The ground also fills what those layers round off.
                        let fill = tint(ground, ground_alpha);
                        if let Some(r) = canvas_viewer.filter(|_| native) {
                            fill_rounded_off(window, r, viewer_radius, viewer_corners, fill);
                        }
                        if let Some(bodies) = &native_bodies {
                            for (_, r) in bodies.borrow().iter() {
                                fill_rounded_off(window, *r, body_radius, [true, true], fill);
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_move_all({
                let listener = cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                    let at = (f32::from(e.position.x), f32::from(e.position.y));
                    let hover = this
                        .geometry
                        .panels
                        .iter()
                        .find(|p| p.rect.contains(at.0, at.1))
                        .map(|p| {
                            let id = this.layout.docks[p.dock].groups[p.group]
                                .active()
                                .to_string();
                            (
                                id,
                                crate::area::Corner::ALL
                                    .into_iter()
                                    .find(|c| c.rect(p.rect).contains(at.0, at.1)),
                            )
                        });
                    if this.area_hover != hover {
                        this.area_hover = hover;
                        cx.notify();
                    }
                    if !e.dragging() {
                        // The button is up: a release that never reached
                        // this window ends the drag here, where it showed.
                        if this.gesture.is_some() {
                            this.end_gesture(cx);
                        }
                        return;
                    }
                    match &mut this.gesture {
                        Some(Gesture::Area(drag)) => {
                            drag.at = e.position;
                            drag.moved |= f32::from(e.position.x - drag.start.x)
                                .hypot(f32::from(e.position.y - drag.start.y))
                                >= 4.;
                            if drag.moved {
                                drag.action = this.layout.area_action(
                                    &this.geometry,
                                    &drag.id,
                                    drag.corner,
                                    at.0,
                                    at.1,
                                    this.tokens.metrics.gap,
                                );
                            }
                            cx.notify();
                        }
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
                            this.layout.resize_with_panel_heights(
                                d,
                                delta,
                                this.dock_bounds,
                                &this.tokens,
                                &this.dock_natural_heights,
                            );
                            cx.notify();
                        }
                        Some(Gesture::Panel(drag)) => {
                            drag.at = e.position;
                            drag.moved |= crate::drag::lifted(
                                (f32::from(drag.start.x), f32::from(drag.start.y)),
                                (f32::from(e.position.x), f32::from(e.position.y)),
                                this.tokens.metrics.panel_drag_threshold,
                            );
                            this.retarget(false);
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
                let listener = cx.listener(|this, e: &MouseUpEvent, _, cx| {
                    if let Some(Gesture::Area(drag)) = &mut this.gesture {
                        drag.at = e.position;
                        if drag.moved {
                            drag.action = this.layout.area_action(
                                &this.geometry,
                                &drag.id,
                                drag.corner,
                                f32::from(e.position.x),
                                f32::from(e.position.y),
                                this.tokens.metrics.gap,
                            );
                        }
                    }
                    this.end_gesture(cx);
                });
                move |e: &MouseUpEvent, phase, _, w, cx| {
                    if phase == DispatchPhase::Bubble && e.button == MouseButton::Left {
                        listener(e, w, cx);
                    }
                }
            })
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| {
                crate::input::noted();
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
                    this.layout.resize_with_panel_heights(
                        &d,
                        delta,
                        this.dock_bounds,
                        &this.tokens,
                        &this.dock_natural_heights,
                    );
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
        // The tabs in the hand (one, or a header's whole stack), and whether
        // they show at a tab position: then they are the drag's indicator.
        let (in_hand, at_tab): (Vec<String>, bool) = match &self.gesture {
            Some(Gesture::Panel(d)) if d.moved && d.live => (
                match d.original.find(&d.id) {
                    Some((dock, group, _)) if d.stack => {
                        d.original.docks[dock].groups[group].tabs.clone()
                    }
                    _ => vec![d.id.clone()],
                },
                matches!(d.shown.map(|p| p.target), Some(DropTarget::Tab { .. })),
            ),
            _ => (vec![], false),
        };
        let lifted_only = match &self.gesture {
            Some(Gesture::Panel(d)) => d.shown.is_none(),
            _ => false,
        };
        let in_slot = !in_hand.is_empty() && !at_tab && !lifted_only;
        self.tab_slides.begin();
        if let Some(integration) = &self.integration {
            integration.open_panels(&self.layout.open_panels(), cx);
        }
        let mut native_bodies = vec![];
        let mut native_clips = vec![];
        let mut native_fades = vec![];
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
            let header_rect = self.geometry.header_rect(&p, &t);
            let header_offset = header_rect.y - p.rect.y;
            // Where the panel shows this frame: its place, plus what is left
            // of its travel there. Its tabs and its extension body follow.
            let shown = self.slides.shown(&id, p.rect);
            let mut panel = positioned(shown)
                .id(SharedString::from(format!("group-{id}")))
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
                // In its slot during a drag the panel is lifted: a soft shadow.
                .elevated(
                    &t,
                    if in_slot && in_hand.contains(&id) {
                        pm_design::Elevation::Menu
                    } else {
                        pm_design::Elevation::Panel
                    },
                )
                // During push-off a header can extend into the dock gap.
                // Its body clips separately; the dock still clips both.
                .when(header_offset <= 0., |p| p.overflow_hidden());
            if body_native && !collapsed {
                // A native body (an extension's web page) takes the pointer
                // where it shows, so it stops short of the dividers' grab.
                let edge = m.border.max(m.splitter_reach);
                let body = native_body(shown, edge, m.header);
                let native_id = self
                    .integration
                    .as_ref()
                    .map_or_else(|| id.to_string(), |i| i.native_panel_id(&id, cx));
                // The slot already includes the dock scroll. Place the body
                // at the animated position, then clip it to the viewport
                // and the travelling sticky header.
                let clip = self.geometry.side_docks.get(&p.dock).map_or(body, |bounds| {
                    let mut clip = body.intersection(*bounds);
                    let bottom = clip.y + clip.h;
                    let header_bottom = header_rect.y + header_rect.h + shown.y - p.rect.y;
                    clip.y = clip.y.max(header_bottom);
                    clip.h = (bottom - clip.y).max(0.);
                    clip
                });
                let (top, bottom) = self.dock_body_fade(p.dock, body);
                native_clips.push((native_id.clone(), clip));
                native_fades.push((native_id.clone(), [f64::from(top), f64::from(bottom)]));
                native_bodies.push((native_id, body));
                // The panel's material everywhere around the page (its header
                // and the edge it leaves the dividers), and in the corners the
                // page rounds off: one surface, as every other panel.
                let (whole, fill) = (shown, tint(c.panel, t.translucency.panel));
                let radius = (m.panel_radius - edge).max(0.);
                panel = panel.child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            for r in layout::outside(whole, &[body]) {
                                window.paint_quad(
                                    PaintQuad::new(Bounds::new(
                                        point(px(r.x), px(r.y)),
                                        gpui::size(px(r.w), px(r.h)),
                                    ))
                                    .background(fill),
                                );
                            }
                            fill_rounded_off(window, body, radius, [true, true], fill);
                        },
                    )
                    .absolute()
                    .size_full(),
                );
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
                .when(
                    (is_viewer && native) || own_ground || header_offset > 0.,
                    |h| h.bg(tint(c.panel, t.translucency.panel)),
                )
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
                    let tab_left;
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
                        tab_left = span_x;
                        span_x += drawn + TAB_SPACING;
                    }
                    // A tab given a new place in its strip slides to it.
                    let slide =
                        self.tab_slides
                            .offset(tab, tab_left + shown.x - p.rect.x, p.rect.y, &t);
                    let held = in_hand.contains(tab);
                    let tabid = tab.clone();
                    let menu_id = tab.clone();
                    let selected = ti == active;
                    // In the hand over no target: dimmed where it was. At a
                    // tab position: the accent pill, the drag's one indicator.
                    let ghost = held && lifted_only;
                    let landing = held && at_tab && multi;
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
                            let s = s
                                .pl(px(close_slot))
                                .pr(px(m.gap))
                                .rounded(px(m.control_radius));
                            if landing {
                                s.bg(rgb(c.accent_wash))
                            } else {
                                s.hover_style(|s| s.bg(rgb(c.hover)))
                            }
                        })
                        .when(slide != 0., |s| s.left(px(slide)))
                        .when(ghost, |s| s.opacity(0.4))
                        .text_color(rgb(if selected || !multi || landing {
                            c.text
                        } else {
                            c.muted
                        }))
                        .when(multi && !selected && !landing, |s| {
                            s.group_hover_style(tab_group.clone(), |s| {
                                s.text_color(rgb(c.secondary))
                            })
                        })
                        .text_size(px(t.typography.title))
                        .font_weight(FontWeight(t.typography.medium_weight))
                        .whitespace_nowrap();
                    // Long titles truncate; every panel retains its tab in the strip.
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
                    if landing {
                        // The strong cue at a tab position: an accent
                        // insertion line at the tab's leading edge.
                        cell = cell.child(
                            div()
                                .absolute()
                                .left(px(0.))
                                .top(px(m.gap / 2.))
                                .bottom(px(m.gap / 2.))
                                .w(px(m.drop_stroke))
                                .rounded(px(m.drop_stroke / 2.))
                                .bg(rgb(c.accent)),
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
                    let entries: Vec<_> = tab_track.overflow.iter()
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
            if let Some((highlighted, start)) = &self.header_wash {
                let progress = start.elapsed().as_secs_f32() / 1.2;
                if highlighted == &id && progress < 1. && !t.accessibility.reduce_motion {
                    header = header.bg(rgb(pm_design::color::mix(
                        c.selected_fill,
                        c.panel,
                        progress,
                    )));
                    w.request_animation_frame();
                }
            }
            // Keep the original interactive header and paint it over the body
            // only while pinned. Unscrolled and folded groups keep normal flow.
            let stuck_header = if header_offset > 0. {
                panel = panel.child(div().h(px(m.header)).flex_none());
                Some(
                    header
                        .absolute()
                        .top(px(header_offset))
                        .left(px(0.))
                        .w_full(),
                )
            } else {
                panel = panel.child(header);
                None
            };
            if collapsed {
                if let Some(bounds) = self.geometry.side_docks.get(&p.dock) {
                    let mut local = shown;
                    local.x -= bounds.x;
                    local.y -= bounds.y;
                    panel = panel.left(px(local.x)).top(px(local.y));
                    root = root.child(positioned(*bounds).overflow_hidden().child(panel));
                } else {
                    root = root.child(panel);
                }
                continue;
            }
            if let (Some(i), Some(f)) = (self.integration.clone(), self.panel_focus.get(&id)) {
                let focused = f.contains_focused(w, cx);
                i.panel_focused(&id, focused, cx);
            }
            let content = self
                .integration
                .as_ref()
                .and_then(|i| i.panel(&id, p.rect, w, cx));
            if let Some(content) = content {
                if let Some(viewport) = self.geometry.side_docks.get(&p.dock) {
                    let height = (p.rect.h - m.header).max(0.);
                    let y = p.rect.y + m.header;
                    let clip_start = header_offset.max(viewport.y - y);
                    let clip_end = height.min(viewport.y + viewport.h - y);
                    let (top, bottom) = self.dock_body_fade(p.dock, Rect { y, h: height, ..p.rect });
                    // Keep the body's layout origin while the sticky header
                    // occludes its first rows. AppKit slots also read this mask.
                    panel = panel.child(
                        div()
                            .relative()
                            .flex_none()
                            .h(px(height))
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top(px(clip_start))
                                    .w_full()
                                    .h(px((clip_end - clip_start).max(0.)))
                                    .overflow_hidden()
                                    .overflow_fade(Edges::<AbsoluteLength> {
                                        top: px(top).into(),
                                        right: px(0.).into(),
                                        bottom: px(bottom).into(),
                                        left: px(0.).into(),
                                    })
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .top(px(-clip_start))
                                            .w_full()
                                            .h(px(height))
                                            .child(content),
                                    ),
                            ),
                    );
                } else {
                    panel = panel.child(content);
                }
            } else {
                let unknown = format!("No installed extension provides “{id}”.");
                let loading = self.integration.as_ref().is_some_and(|i| i.panels_loading(cx));
                let coming = !panels.contains(&id) && self.awaiting_panels && self.integration.is_some();
                let (heading, line, action) = if loading && !layout::PANELS.contains(&id.as_str()) {
                    ("Loading…", "", None)
                } else if panels.contains(&id) {
                    layout::empty_state(&id)
                } else if coming {
                    ("", "", None)
                } else {
                    ("Panel Unavailable", unknown.as_str(), None)
                };
                let mut empty = empty_state(&t, heading, line)
                    .h(px((p.rect.h - m.header).max(0.)))
                    .overflow_hidden();
                // A panel still on its way (its extension starting): the
                // loading state under its own title, never an empty body.
                if coming {
                    static SINCE: std::sync::OnceLock<std::time::Instant> =
                        std::sync::OnceLock::new();
                    let turns = SINCE
                        .get_or_init(std::time::Instant::now)
                        .elapsed()
                        .as_secs_f64();
                    empty = empty.child(crate::welcome_view::spinner(
                        if t.accessibility.reduce_motion {
                            0.
                        } else {
                            turns
                        },
                        &t,
                    ));
                    w.request_animation_frame();
                }
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
            if let Some(header) = stuck_header {
                panel = panel.child(header);
            }
            if let Some(bounds) = self.geometry.side_docks.get(&p.dock) {
                let mut local = shown;
                local.x -= bounds.x;
                local.y -= bounds.y;
                panel = panel.left(px(local.x)).top(px(local.y));
                root = root.child(positioned(*bounds).overflow_hidden().child(panel));
            } else {
                root = root.child(panel);
            }
        }
        // Electron v1.2.0 hides side-column scrollbar chrome. Wheel and
        // momentum still move the column; no idle scrollbar timer is needed.
        self.geometry.tabs = tab_spans;
        if self.tab_slides.end() {
            w.request_animation_frame();
        }
        if let Some(integration) = &self.integration {
            // Every panel on screen, built-in or an extension's: the viewer
            // and the timeline are not native bodies but are shown.
            let shown: Vec<&str> = self.panel_order.iter().map(String::as_str).collect();
            integration.shown_panels(&shown, cx);
        }
        self.native_body_rects = native_bodies.clone();
        if let Some(h) = &self.host {
            *h.borrow().native_panel_clips.borrow_mut() = native_clips;
            *h.borrow().native_panel_fades.borrow_mut() = native_fades;
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
            let mut splitter = positioned(hit)
                .id(format!("splitter-{i}"))
                .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, window, cx| {
                    this.dock_wheel(e, window, cx)
                }))
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
                    this.layout.resize_with_panel_heights(
                        &key_divider,
                        delta,
                        this.dock_bounds,
                        &this.tokens,
                        &this.dock_natural_heights,
                    );
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
                            if this.layout.reset_dock(&divd)
                                || this.layout.reset_split_with_panel_heights(
                                    &divd,
                                    this.dock_bounds,
                                    &this.tokens,
                                    &this.dock_natural_heights,
                                )
                            {
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
                );
            if d.group.is_some()
                && let Some(bounds) = self.geometry.side_docks.get(&d.dock)
            {
                splitter = splitter
                    .left(px(hit.x - bounds.x))
                    .top(px(hit.y - bounds.y));
                root = root.child(positioned(*bounds).overflow_hidden().child(splitter));
            } else {
                root = root.child(splitter);
            }
        }
        if self.maximized_panel.is_none() && !dragging {
            for p in self.geometry.panels.clone() {
                let id = self.layout.docks[p.dock].groups[p.group]
                    .active()
                    .to_string();
                let panel_hover = self.area_hover.as_ref().is_some_and(|(h, _)| h == &id);
                for (i, corner) in crate::area::Corner::ALL.into_iter().enumerate() {
                    let corner_rect = self.geometry.side_docks.get(&p.dock)
                        .map_or(corner.rect(p.rect), |clip| corner.rect(p.rect).intersection(*clip));
                    if corner_rect.w <= 0. || corner_rect.h <= 0. { continue; }
                    let active = self
                        .area_hover
                        .as_ref()
                        .is_some_and(|(h, c)| h == &id && *c == Some(corner));
                    let goal = if active {
                        1.
                    } else if panel_hover {
                        0.7
                    } else {
                        0.
                    };
                    let fade = w
                        .use_keyed_transition(
                            format!("area-fade-{id}-{i}"),
                            cx,
                            Duration::from_millis(100),
                            |_, _| goal,
                        )
                        .with_easing(|p| 1. - (1. - p).powi(3));
                    fade.update(cx, |value, _| *value = goal);
                    let opacity = *fade.evaluate(w, cx);
                    let ink = if active {
                        tint(c.accent, opacity)
                    } else {
                        tint(c.text, 0.34 * opacity)
                    };
                    let id_down = id.clone();
                    root = root.child(
                        positioned(corner_rect)
                            .id(format!("area-{id}-{i}"))
                            .tooltip(tooltip(&t, "Drag to split, join or dock"))
                            .cursor(CursorStyle::Crosshair)
                            .child(
                                canvas(
                                    move |_, _, _| (),
                                    move |b, _, window, _| {
                                        let x = f32::from(b.origin.x) + 2.;
                                        let y = f32::from(b.origin.y) + 2.;
                                        // CSS 7px square with a diagonal 30% (34% on hover) stop.
                                        let leg = 7. * 2. * if active { 0.34 } else { 0.30 };
                                        let x = if corner.left() { x } else { x + 7. };
                                        let y = if corner.top() { y } else { y + 7. };
                                        let mut path = PathBuilder::fill();
                                        path.move_to(point(px(x), px(y)));
                                        path.line_to(point(
                                            px(x + if corner.left() { leg } else { -leg }),
                                            px(y),
                                        ));
                                        path.line_to(point(
                                            px(x),
                                            px(y + if corner.top() { leg } else { -leg }),
                                        ));
                                        path.close();
                                        if let Ok(path) = path.build() {
                                            window.paint_path(path, ink);
                                        }
                                    },
                                )
                                .size_full(),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.gesture = Some(Gesture::Area(AreaDrag {
                                        id: id_down.clone(),
                                        corner,
                                        start: e.position,
                                        at: e.position,
                                        original: this.layout.clone(),
                                        moved: false,
                                        action: crate::area::Action::None,
                                    }));
                                    this.watch_release(cx);
                                    window.focus(&this.focus, cx);
                                    cx.notify();
                                }),
                            ),
                    );
                }
            }
        }
        if let Some(Gesture::Area(drag)) = &self.gesture
            && drag.moved
        {
            let action = &drag.action;
            let target = match action {
                crate::area::Action::Join { target } | crate::area::Action::Dock { target, .. } => {
                    target
                }
                _ => &drag.id,
            };
            if let Some((d, g, _)) = self.layout.find(target)
                && let Some(p) = self
                    .geometry
                    .panels
                    .iter()
                    .find(|p| (p.dock, p.group) == (d, g))
            {
                let mut r = p.rect;
                let label = match action {
                    crate::area::Action::Split { top, height } => {
                        r.h = *height;
                        if !top {
                            r.y = p.rect.y + p.rect.h - height;
                        }
                        "Split area"
                    }
                    crate::area::Action::Join { .. } => "Join areas",
                    crate::area::Action::Dock { place, .. } => {
                        if *place != crate::area::Place::Replace {
                            r.h /= 2.;
                            if *place == crate::area::Place::Below {
                                r.y += r.h;
                            }
                        }
                        "Move area here"
                    }
                    _ => "Drag further to split",
                };
                if matches!(action, crate::area::Action::Join { .. }) {
                    let source_y = self
                        .layout
                        .find(&drag.id)
                        .and_then(|(d, g, _)| {
                            self.geometry
                                .panels
                                .iter()
                                .find(|p| (p.dock, p.group) == (d, g))
                        })
                        .map_or(0., |p| p.rect.y);
                    root = root.child(
                        positioned(r)
                            .bg(tint(0x000000, 0.46))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(34.))
                            .text_color(tint(0xffffff, 0.82))
                            .child(if r.y > source_y { "↓" } else { "↑" }),
                    );
                } else if !matches!(action, crate::area::Action::None) {
                    root = root.child(
                        positioned(r)
                            .bg(tint(c.accent, 0.22))
                            .border_2()
                            .border_color(tint(c.accent, 0.7)),
                    );
                    if let crate::area::Action::Split { top, .. } = action {
                        let y = if *top {
                            r.y + r.h + m.gap / 2.
                        } else {
                            r.y - m.gap / 2.
                        };
                        root = root.child(
                            positioned(Rect {
                                x: r.x,
                                y: y - 1.,
                                w: r.w,
                                h: 2.,
                            })
                            .bg(rgb(c.accent)),
                        );
                    }
                }
                root = root.child(
                    div()
                        .absolute()
                        .left(px((f32::from(drag.at.x) + 14.)
                            .clamp(8., (f32::from(viewport.width) - 200.).max(8.))))
                        .top(px((f32::from(drag.at.y) + 14.)
                            .clamp(8., (f32::from(viewport.height) - 50.).max(8.))))
                        .px(px(m.inset))
                        .py(px(m.gap))
                        .rounded(px(m.control_radius))
                        .bg(rgb(c.panel))
                        .child(label),
                );
            }
        }
        if matches!(&self.gesture, Some(Gesture::Area(d)) if d.moved) {
            root = root.child(div().absolute().size_full().cursor(CursorStyle::Crosshair));
        }
        // The drag's one indicator for a column position (a tab position's
        // is the tab itself, above) and the chip beside the pointer. Neither
        // takes the pointer. Work-surface overlays stay solid tints (no
        // glass below the controls band).
        let o = crate::drag::CHIP_OFFSET;
        let mut chip: Option<(String, (f32, f32), f32)> = None;
        let mut slot: Option<(Rect, f32)> = None;
        if let Some(Gesture::Panel(drag)) = &self.gesture
            && drag.moved
        {
            let since = drag.lifted.unwrap_or_else(crate::drag::now);
            let shown = crate::drag::progress(since, crate::drag::LIFT, &t);
            if shown < 1. {
                w.request_animation_frame();
            }
            chip = Some((
                self.moving_name(&drag.original, &drag.id, drag.stack, &panels),
                (f32::from(drag.at.x) + o, f32::from(drag.at.y) + o),
                shown,
            ));
            slot = if drag.live {
                // The panel is in its slot already: the slot is where it is.
                drag.shown
                    .filter(|p| !matches!(p.target, DropTarget::Tab { .. }))
                    .and_then(|_| self.slot_rect(&drag.id))
                    .map(|r| (r, 1.))
            } else {
                // From outside: where it would dock.
                drag.preview.map(|p| (p.rect, 1.))
            };
        } else if let Some(settle) = self.settle.clone() {
            // After the release: the chip glides to where the panel landed
            // (or home) as it fades, and the slot's wash fades with it.
            let done = crate::drag::progress(settle.since, crate::drag::SETTLE, &t);
            if done >= 1. {
                self.settle = None;
            } else {
                let at = (
                    settle.from.0 + (settle.to.0 - settle.from.0) * done,
                    settle.from.1 + (settle.to.1 - settle.from.1) * done,
                );
                chip = Some((settle.name, at, 1. - done));
                slot = settle
                    .slot
                    .and_then(|id| self.slot_rect(&id))
                    .map(|r| (r, 1. - done));
                w.request_animation_frame();
            }
        }
        if let Some((r, alpha)) = slot {
            // Calm: the real panel on its own surface (lifted, above), a
            // hairline accent ring and the faintest accent tint. Increase
            // Contrast keeps the full drop stroke.
            let contrast = t.accessibility.increase_contrast;
            root = root.child(
                positioned(r)
                    .id("drop-preview")
                    .rounded(px(m.panel_radius))
                    .border(px(if contrast { m.drop_stroke } else { m.border.max(1.) }))
                    .border_color(tint(c.accent, if contrast { 1. } else { crate::drag::SLOT_RING }))
                    .bg(tint(c.accent, crate::drag::SLOT_TINT))
                    .opacity(alpha),
            );
        }
        if let Some((name, at, alpha)) = chip {
            // Wholly inside the window, however long the name.
            let width =
                title_width(&name, t.typography.title, w) + 2. * (m.gap + 2.) + m.gap;
            let (x, y) = crate::drag::chip_origin(
                at,
                (width, m.hit),
                (f32::from(viewport.width), f32::from(viewport.height)),
            );
            root = root.child(
                div()
                    .id("drag-ghost")
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .h(px(m.hit))
                    .px(px(m.gap + 2.))
                    .flex()
                    .items_center()
                    .rounded(px(m.control_radius))
                    .bg(rgb(c.floating))
                    .elevated(&t, pm_design::Elevation::Menu)
                    .opacity(alpha)
                    .text_size(px(t.typography.title))
                    .font_weight(FontWeight(t.typography.semibold_weight))
                    .whitespace_nowrap()
                    .child(name),
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
        root = root.child(Self::titlebar_band(top, self.tokens.colors.window));
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

/// Paints `color` in what a native layer filling `r` rounds off its bottom
/// corners (`[left, right]`): continuous corners of `radius`, as the layer's.
fn fill_rounded_off(
    window: &mut Window,
    r: Rect,
    radius: f32,
    [left, right]: [bool; 2],
    color: impl Into<Background> + Copy,
) {
    use pm_design::corner::{continuous_corner, fit};
    let radius = fit(radius as f64, r.w as f64, r.h as f64);
    if radius <= 0. {
        return;
    }
    let bottom = (r.y + r.h) as f64;
    let at = |(x, y): (f64, f64)| point(px(x as f32), px(y as f32));
    for (on, corner, along) in [
        (left, (r.x as f64, bottom), (1., 0.)),
        (right, ((r.x + r.w) as f64, bottom), (-1., 0.)),
    ] {
        if !on {
            continue;
        }
        let (start, curves) = continuous_corner(corner, along, (0., -1.), radius);
        let mut path = PathBuilder::fill();
        path.move_to(at(curves[2][2]));
        path.line_to(at(corner));
        path.line_to(at(start));
        for [a, b, to] in curves {
            path.cubic_bezier_to(at(to), at(a), at(b));
        }
        path.close();
        if let Ok(path) = path.build() {
            window.paint_path(path, color);
        }
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

    #[gpui::test]
    fn area_corners_capture_pointer_and_escape_restores_layout(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            Editor::new(
                Tokens::new(pm_design::Scheme::Dark, Default::default()),
                None,
                cx,
            )
        });
        cx.run_until_parked();
        let original = view.read_with(cx, |v, _| v.layout.clone());
        let r = view.read_with(cx, |v, _| v.geometry.panels[0].rect);
        for corner in crate::area::Corner::ALL {
            let hit = corner.rect(r);
            let at = point(px(hit.x + 5.), px(hit.y + 5.));
            cx.simulate_mouse_move(at, None, Modifiers::none());
            cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
            view.read_with(cx, |v, _| {
                assert!(matches!(&v.gesture,Some(Gesture::Area(d)) if d.corner==corner))
            });
            cx.simulate_mouse_move(
                point(px(r.x + 20.), px(r.y + r.h / 2.)),
                MouseButton::Left,
                Modifiers::none(),
            );
            view.update(cx, |v, cx| v.cancel_gesture(cx));
            cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::none());
            view.read_with(cx, |v, _| assert_eq!(v.layout, original));
        }
    }
    #[::core::prelude::v1::test]
    fn area_corners_headless_capture() {
        let Some(dir) = std::env::var_os("PM_PANEL_CORNERS_CAPTURE_DIR") else {
            return;
        };
        objc2::rc::autoreleasepool(|_| {
            let mut cx = HeadlessAppContext::with_platform(
                Arc::new(gpui_parley::ParleyTextSystem::new(
                    gpui_parley::SystemFonts::Load,
                )),
                Arc::new(()),
                || {
                    Some(Box::new(
                        gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                    ))
                },
            );
            let w = cx
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
            let (id, r) = w
                .update(&mut cx, |v, _, _| {
                    let p = &v.geometry.panels[0];
                    (
                        v.layout.docks[p.dock].groups[p.group].active().to_string(),
                        p.rect,
                    )
                })
                .unwrap();
            for (name, hover) in [("rest", None), ("panel-hover", Some((id.clone(), None)))]
                .into_iter()
                .chain(
                    crate::area::Corner::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(i, c)| {
                            (
                                [
                                    "top-left-hover",
                                    "top-right-hover",
                                    "bottom-left-hover",
                                    "bottom-right-hover",
                                ][i],
                                Some((id.clone(), Some(c))),
                            )
                        }),
                )
            {
                w.update(&mut cx, |v, _, cx| {
                    v.area_hover = hover;
                    cx.notify();
                })
                .unwrap();
                cx.run_until_parked();
                // Settle the finite opacity transition before each reference frame.
                std::thread::sleep(Duration::from_millis(120));
                w.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
                cx.run_until_parked();
                let image = cx.capture_screenshot(w.into()).unwrap();
                let scale = image.width() as f32 / 1440.;
                let path = std::path::Path::new(&dir);
                image.save(path.join(format!("native-{name}.png"))).unwrap();
                for (i, c) in crate::area::Corner::ALL.into_iter().enumerate() {
                    let hit = c.rect(r);
                    let x = ((hit.x - 6.).max(0.) * scale) as u32;
                    let y = ((hit.y - 6.).max(0.) * scale) as u32;
                    image::imageops::crop_imm(
                        &image,
                        x,
                        y,
                        (28. * scale) as u32,
                        (28. * scale) as u32,
                    )
                    .to_image()
                    .save(path.join(format!("native-{name}-corner-{i}.png")))
                    .unwrap();
                }
            }
        });
    }

    #[::core::prelude::v1::test]
    fn canvas_press_releases_control_focus_without_a_panel_ring() {
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
                w.focus(&v.workspace_focus, cx);
                v.ringed_panel = Some("timeline".into());
                v.focus_canvas(w, cx);
                assert!(v.panel_focus["viewer"].is_focused(w));
                assert!(v.ringed_panel.is_none());
                assert!(v.split_focus.is_none());
            })
            .unwrap();
    }

    fn command(name: &str) -> Command {
        Command {
            name: name.into(),
            args: vec![],
            token: None,
        }
    }

    #[::core::prelude::v1::test]
    fn library_reveal_respects_reduce_motion_and_reports_errors() {
        for reduced in [false, true] {
            let mut cx = app();
            let window = cx
                .open_window(size(px(1000.), px(600.)), |_, cx| {
                    cx.new(|cx| {
                        Editor::new(
                            Tokens::new(
                                pm_design::Scheme::Dark,
                                pm_design::Accessibility {
                                    reduce_motion: reduced,
                                    ..Default::default()
                                },
                            ),
                            None,
                            cx,
                        )
                    })
                })
                .unwrap();
            window
                .update(&mut cx, |v, _, cx| {
                    assert!(v.show_library_panel("notes", cx).is_ok());
                    assert_eq!(v.reveal_panel.as_deref(), Some("notes"));
                    assert_eq!(v.header_wash.is_none(), reduced);
                    let place = v.layout.find("notes").unwrap();
                    v.shell_command("panel:close:notes", cx);
                    assert!(v.show_library_panel("notes", cx).is_ok());
                    assert_eq!(v.layout.find("notes").unwrap(), place);
                    let (dock, group, _) = v.layout.find("assets").unwrap();
                    v.layout
                        .move_panel_to(
                            "notes",
                            DropTarget::Tab {
                                dock,
                                group,
                                index: 1,
                            },
                        )
                        .unwrap();
                    v.shell_command("panel:close:notes", cx);
                    v.layout.docks[dock].collapsed = true;
                    v.layout.docks[dock].groups[group].collapsed = true;
                    assert!(v.show_library_panel("notes", cx).is_ok());
                    let (di, gi, ti) = v.layout.find("notes").unwrap();
                    assert_eq!((di, gi), (dock, group));
                    assert!(!v.layout.docks[di].collapsed);
                    assert!(!v.layout.docks[di].groups[gi].collapsed);
                    assert_eq!(v.layout.docks[di].groups[gi].active, ti);
                    let before = v.layout.clone();
                    assert!(v.show_library_panel("missing-panel", cx).is_err());
                    assert_eq!(v.layout, before);
                })
                .unwrap();
        }
    }

    #[::core::prelude::v1::test]
    fn panel_build_slot_stays_transient_and_handoff_preserves_geometry() {
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
        window
            .update(&mut cx, |v, _, cx| {
                v.reserve_agent_panel(
                    "test-build",
                    &serde_json::json!({"beside":"inspector"}),
                    280.,
                );
                let (dock, group, tab) = v.layout.find("test-build").unwrap();
                let before = v.layout.docks[dock].groups[group].clone();
                v.save();
                assert!(!v.record.to_string().contains("test-build"));
                v.layout.docks[dock].groups[group].tabs[tab] = "test-final".into();
                v.release_agent_panel("test-build");
                v.commit_agent_panel_layout(cx);
                let (d, g, _) = v.layout.find("test-final").unwrap();
                assert_eq!((d, g), (dock, group));
                assert_eq!(v.layout.docks[d].groups[g].size, before.size);
                assert!(v.record.to_string().contains("test-final"));
                assert!(!v.record.to_string().contains("test-build"));
            })
            .unwrap();
    }

    #[::core::prelude::v1::test]
    fn p4_explicit_close_releases_build_slot() {
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
        window
            .update(&mut cx, |v, _, cx| {
                v.reserve_agent_panel(
                    "panel-build-close",
                    &serde_json::json!({"tab_in":"inspector"}),
                    280.,
                );
                assert!(v.shell_command("panel:close:panel-build-close", cx));
                assert!(!v.agent_build_slots.contains_key("panel-build-close"));
                assert!(v.layout.find("panel-build-close").is_none());
            })
            .unwrap();
    }
    #[::core::prelude::v1::test]
    fn p5_save_agent_layout_never_persists_build_ids() {
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
        window
            .update(&mut cx, |v, _, cx| {
                v.reserve_agent_panel(
                    "panel-build-save",
                    &serde_json::json!({"tab_in":"inspector"}),
                    280.,
                );
                v.save_agent_layout("Synthetic slots", v.layout.docks.clone(), cx)
                    .unwrap();
                assert!(!v.record.to_string().contains("panel-build-save"));
                // Every other persistent writer uses this same serialization boundary.
                v.reserve_agent_panel(
                    "panel-build-copy",
                    &serde_json::json!({"tab_in":"inspector"}),
                    280.,
                );
                assert!(
                    !workspace::write(&v.layout, &v.record)
                        .to_string()
                        .contains("panel-build-copy")
                );
                v.shell_command("layout:save:Synthetic copy", cx);
                assert!(!v.record.to_string().contains("panel-build-copy"));
                let mut raw = v.record.clone();
                raw["layout"]["docks"][0]["panels"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({"id":"panel-build-injected"}));
                v.commit(raw);
                assert!(!v.record.to_string().contains("panel-build-injected"));
            })
            .unwrap();
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
                // An extension's panel opening again as it restarts (PM-427):
                // in its stack, it never takes the showing tab, parked or not.
                for id in ["ext.voice", "inspector"] {
                    assert!(v.shell_command(&format!("panel:restore:{id}"), cx));
                    assert_eq!(showing(v), "ext.pexels", "{id} restored in place");
                }
                assert!(v.shell_command("panel:park:ext.voice", cx));
                assert!(v.shell_command("panel:restore:ext.voice", cx));
                assert!(v.layout.find("ext.voice").is_some(), "back in its place");
                assert_eq!(showing(v), "ext.pexels");
                assert!(v.shell_command("panel:background-close:ext.pexels", cx));
                assert_eq!(showing(v), "ext.pexels", "a background close keeps the front tab");
                v.show_library_panel("ext.voice", cx).unwrap();
                assert_eq!(showing(v), "ext.voice", "Window › Panels still reveals the chosen tab");
            })
            .unwrap();
    }
    #[::core::prelude::v1::test]
    fn late_panels_keep_separate_groups_sizes_and_active_tabs() {
        let mut cx = app();
        let window = cx.open_window(size(px(1440.), px(900.)), |_, cx| {
            cx.new(|cx| Editor::new(Tokens::new(pm_design::Scheme::Dark, Default::default()), None, cx))
        }).unwrap();
        window.update(&mut cx, |v, _, cx| {
            v.integration = Some(Rc::new(Contributions(vec![
                ("ext.one".into(), "One".into()), ("ext.two".into(), "Two".into())
            ])));
            v.show_record(serde_json::json!({"id":"custom", "layout":{"docks":[
                {"id":"center","panels":[{"id":"viewer","flex":true}]},
                {"id":"right","size":340,"panels":[
                    {"id":"ext.one","size":230}, {"id":"inspector","size":210},
                    {"id":"ext.two","tabbed":true,"active":true}]}]}}));
            let original = v.layout.clone();
            for id in ["ext.one", "ext.two"] {
                assert!(v.shell_command(&format!("panel:park:{id}"), cx));
            }
            assert_eq!(v.layout, original);
            for id in ["ext.two", "ext.one"] {
                assert!(v.shell_command(&format!("panel:registered:{id}"), cx));
            }
            assert_eq!(v.layout, original);
            assert_eq!(workspace::layout(&v.record), original);
            assert!(v.shell_command("panel:park:ext.one", cx));
            assert!(v.shell_command("panel:close:ext.one", cx));
            let closed = v.layout.clone();
            assert!(v.shell_command("panel:registered:ext.one", cx));
            assert_eq!(v.layout, closed, "registration does not reopen a closed panel");
            assert!(v.shell_command("panel:park:ext.two", cx));
            v.bind_project("another-project".into());
            v.show_record(workspace::preset("review").unwrap());
            v.save();
            assert!(v.shell_command("panel:registered:ext.two", cx));
            assert!(v.layout.find("ext.two").is_none());
            assert!(v.layout.find("ext.one").is_none());
            assert!(workspace::layout(&v.record).find("ext.one").is_none(),
                "a suspended panel from another project cannot enter this workspace");
        }).unwrap();
    }

    /// A project whose saved stack shows an extension's panel that is not
    /// there at open (uninstalled, or its realm still starting): the saved
    /// choice remains visible as a placeholder until its owner is ready.
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
                assert_eq!(showing(&v.layout), "ext.voice", "the placeholder keeps its saved tab");
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
        /// The project the window holds (`None`: "p"); `home`: none.
        project: RefCell<Option<String>>,
        home: std::cell::Cell<bool>,
        contributed: RefCell<Vec<(String, String)>>,
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
        fn contributed_panels(&self, _: &App) -> Vec<(String, String)> {
            self.0.contributed.borrow().clone()
        }
        fn project_id(&self, _: &App) -> Option<String> {
            (!self.0.home.get())
                .then(|| self.0.project.borrow().clone().unwrap_or_else(|| "p".into()))
        }
        fn status(&self, _: &App) -> String {
            String::new()
        }
        fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    }

    /// Acceptance run 24: a workspace naming a panel from an extension not
    /// installed here. Its named placeholder keeps its saved place, even if
    /// initial extension registration finishes without providing it.
    #[gpui::test]
    fn a_panel_no_extension_provides_keeps_its_named_placeholder(cx: &mut TestAppContext) {
        const ID: &str = "background-remover.panel";
        let mut mine = workspace::preset("design").unwrap();
        mine["id"] = "mine".into();
        mine["name"] = "Mine".into();
        mine["builtin"] = false.into();
        mine["layout"]["docks"] = serde_json::json!([
            {"flex": true, "id": "center", "panels": [{"flex": true, "id": "viewer"}]},
            {"id": "right", "size": 325, "panels": [
                {"id": "inspector", "flex": true},
                {"id": ID, "tabbed": true, "active": true}
            ]}
        ]);
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
            e.project = "p".into();
            e
        });
        window
            .update(cx, |v, _, cx| {
                v.sync_workspaces(cx);
                v.settle_absent_panels(cx);
                assert!(v.layout.open_panels().contains(&ID), "waits while extensions load");
                assert_eq!(v.panels(cx).title(ID), "Background Remover");
                let layout = v.layout.clone();
                assert!(v.run_shell_command("panels:ready", cx));
                assert!(!v.panels(cx).contains(ID), "no extension provides the panel");
                assert!(v.absent.contains(ID), "the placeholder remains unavailable");
                assert_eq!(v.layout, layout, "its saved place and active tab remain");
                assert!(v.layout.open_panels().contains(&"inspector"));
                v.save();
                let saved = serde_json::to_string(&v.record).unwrap();
                assert!(saved.contains(ID), "the workspace keeps it: {saved}");
                // Its extension is installed: it is back where it was.
                *kernel.contributed.borrow_mut() = vec![(ID.into(), "Background Remover".into())];
                v.settle_absent_panels(cx);
                assert!(!v.absent.contains(ID));
                assert_eq!(v.layout, layout);
                let (dock, group, _) = v.layout.find(ID).expect("back");
                assert!(v.layout.docks[dock].groups[group].tabs.iter().any(|t| t == "inspector"));
            })
            .unwrap();
    }

    /// Acceptance run 24: a new tab (Home until its project opens) shows the
    /// last used workspace, an edited Design with its timeline collapsed. A
    /// layout write it queued then, or one made before the store hears of
    /// the project the tab took, never becomes that new project's
    /// workspace: the project opens with Design as shipped (PM-454).
    #[gpui::test]
    fn a_new_tab_never_files_the_layout_it_showed_as_home_under_its_new_project(
        cx: &mut TestAppContext,
    ) {
        let mut edited = workspace::preset("design").unwrap();
        edited["id"] = "wsedited".into();
        edited["name"] = "Design (edited)".into();
        edited["builtin"] = false.into();
        edited["layout"]["docks"] =
            serde_json::json!([{"id": "center", "size": 300, "panels": ["viewer"]}]);
        let kernel = Rc::new(Kernel::default());
        kernel.home.set(true);
        let home = workspace::Store {
            records: workspace::seeded(std::slice::from_ref(&edited)),
            current: Some("wsedited".into()),
            default: None,
            project: None,
            project_id: None,
            trashed: vec![],
        };
        *kernel.store.borrow_mut() = Rc::new(home.clone());
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let stub = kernel.clone();
        let window = cx.add_window(|_, cx| {
            let mut e = Editor::new(tokens, None, cx);
            e.integration = Some(Rc::new(Stub(stub)));
            e
        });
        let resized = |record: &serde_json::Value, size: u32| {
            let mut r = record.clone();
            r["layout"]["docks"][0]["size"] = size.into();
            r
        };
        let active_ids = |kernel: &Kernel| -> Vec<String> {
            kernel
                .writes
                .borrow()
                .iter()
                .filter_map(|w| match w {
                    workspace::Write::Active { after, .. } => {
                        workspace::record_id(after).map(str::to_owned)
                    }
                    _ => None,
                })
                .collect()
        };
        window
            .update(cx, |v, _, cx| {
                v.sync_workspaces(cx);
                assert_eq!(v.layout.workspace, "wsedited", "Home follows the last used");
                // A layout change queued while the tab is Home…
                let r = resized(&v.record, 320);
                v.commit(r);
                // …and the tab takes a new project before the next frame.
                kernel.home.set(false);
                *kernel.project.borrow_mut() = Some("new".into());
                v.bind_project("new".into());
                // A change before the store hears of the project.
                let r = resized(&v.record, 340);
                v.commit(r);
                v.sync_workspaces(cx);
                assert!(active_ids(&kernel).is_empty(), "{:?}", kernel.writes.borrow());
                // The store's answer: a project with no workspace of its own.
                *kernel.store.borrow_mut() = Rc::new(workspace::Store {
                    project_id: Some("new".into()),
                    ..home.clone()
                });
                v.sync_workspaces(cx);
                let design = workspace::preset("design").unwrap();
                assert_eq!(v.workspace_record(), &design);
                v.sync_workspaces(cx);
                assert!(matches!(kernel.writes.borrow().as_slice(), [
                    workspace::Write::Active { before, after }
                ] if before.is_null() && *after == design));
                *kernel.store.borrow_mut() = Rc::new(workspace::Store {
                    project_id: Some("new".into()),
                    project: Some(design),
                    ..home.clone()
                });
                v.sync_workspaces(cx);
                assert!(v.pending.is_empty());
                // From here on its own layout changes are its own.
                let r = resized(&v.record, 360);
                v.commit(r.clone());
                v.sync_workspaces(cx);
                assert!(matches!(kernel.writes.borrow().as_slice(), [
                    workspace::Write::Active { .. },
                    workspace::Write::Active { after, .. }
                ] if *after == r));
            })
            .unwrap();
    }

    /// Acceptance run 22: a window that showed an edited copy of Design (its
    /// timeline collapsed) takes a new project, which has no workspace of
    /// its own: the project starts with Design as shipped (PM-454).
    #[gpui::test]
    fn a_new_project_in_a_window_never_takes_the_windows_last_workspace(cx: &mut TestAppContext) {
        let mut edited = workspace::preset("design").unwrap();
        edited["id"] = "wsedited".into();
        edited["name"] = "Design (edited)".into();
        edited["builtin"] = false.into();
        edited["layout"]["docks"] = serde_json::json!([{"id": "center", "panels": ["viewer"]}]);
        let kernel = Rc::new(Kernel::default());
        *kernel.store.borrow_mut() = Rc::new(workspace::Store {
            records: workspace::seeded(std::slice::from_ref(&edited)),
            current: Some("wsedited".into()),
            default: None,
            project: Some(edited.clone()),
            project_id: Some("old".into()),
            trashed: vec![],
        });
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let stub = kernel.clone();
        let window = cx.add_window(|_, cx| {
            let mut e = Editor::new(tokens, None, cx);
            e.integration = Some(Rc::new(Stub(stub)));
            e.restore(&crate::session::WindowState {
                id: 1,
                frame: [0., 0., 1440., 900.],
                project: "old".into(),
                workspace: "wsedited".into(),
                tab_group: None,
                tab_index: 0,
                pages: vec![],
                fullscreen: false,
                name: None,
            });
            e
        });
        window
            .update(cx, |v, _, cx| {
                v.sync_workspaces(cx);
                assert_eq!(v.layout.workspace, "wsedited", "its own project keeps it");
                // The window takes a new project with no workspace of its own.
                let before = (**kernel.store.borrow()).clone();
                *kernel.store.borrow_mut() = Rc::new(workspace::Store {
                    project: None,
                    project_id: Some("new".into()),
                    ..before
                });
                v.sync_workspaces(cx);
                assert_eq!(v.workspace_record(), &workspace::preset("design").unwrap());
            })
            .unwrap();
    }

    #[gpui::test]
    fn inherited_workspace_is_written_without_a_layout_edit(cx: &mut TestAppContext) {
        let kernel = Rc::new(Kernel::default());
        *kernel.project.borrow_mut() = Some("old-project".into());
        *kernel.store.borrow_mut() = Rc::new(workspace::Store {
            records: workspace::seeded(&[]),
            current: Some("review".into()),
            project_id: Some("old-project".into()),
            project: Some(serde_json::Value::Null),
            ..Default::default()
        });
        let stub = kernel.clone();
        let window = cx.add_window(|_, cx| {
            let mut e = Editor::new(Tokens::new(pm_design::Scheme::Dark, Default::default()), None, cx);
            e.preferred = Some("review".into());
            e.preferred_for = Some("old-project".into());
            e.integration = Some(Rc::new(Stub(stub)));
            e
        });
        window.update(cx, |v, _, cx| {
            v.sync_workspaces(cx);
            assert_eq!(v.layout.workspace, "review");
            v.sync_workspaces(cx);
            v.sync_workspaces(cx);
            let writes = kernel.writes.borrow();
            assert!(matches!(&writes[..], [workspace::Write::Active { before, after }]
                if before.is_null() && workspace::record_id(after) == Some("review")));
            let workspace::Write::Active { after, .. } = writes[0].clone() else { panic!() };
            drop(writes);
            let old = (**kernel.store.borrow()).clone();
            *kernel.store.borrow_mut() = Rc::new(workspace::Store {
                project: Some(after), current: Some("design".into()), ..old
            });
            v.sync_workspaces(cx);
            assert!(v.pending.is_empty(), "the actual project publication acknowledges the snapshot");
            assert_eq!(v.layout.workspace, "review");
        }).unwrap();
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

    #[gpui::test]
    fn invalid_outside_drop_does_not_save_a_fallback_group(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(
            Tokens::new(pm_design::Scheme::Dark, Default::default()), None, cx));
        view.update(cx, |v, cx| {
            let before = v.layout.clone();
            let at = gpui::point(gpui::px(0.), gpui::px(0.));
            let mut drag = v.new_drag("agent", at, false, false);
            drag.moved = true;
            drag.preview = Some(crate::layout::DropPreview {
                target: crate::layout::DropTarget::Tab { dock: usize::MAX, group: 0, index: 0 },
                rect: crate::layout::Rect::default(),
                near: crate::layout::Near::Nothing,
            });
            v.gesture = Some(super::Gesture::Panel(drag));
            assert!(!v.drop_from_outside("agent", cx));
            assert_eq!(v.layout, before);
            assert!(v.outbox.is_empty());
        });
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
        // Exercise input recovery at rest, independently of layout animation.
        let tokens = Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility {
            reduce_motion: true,
            ..Default::default()
        });
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        // The panels' travel after a drop runs on this clock: each press
        // below is made once they have arrived, on the header it names.
        crate::drag::set_frame_time(Some(std::time::Instant::now()));
        let arrive = |cx: &mut gpui::VisualTestContext| {
            for _ in 0..4 {
                crate::drag::advance_frame_time(0.25);
                view.update(cx, |_, cx| cx.notify());
                cx.run_until_parked();
            }
        };
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
        arrive(cx);
        let landed = view.read_with(cx, |v, _| v.layout.clone());
        // New content floors may place Effects below the side viewport.
        // Reveal its header before starting another real pointer drag.
        view.update(cx, |v, cx| {
            let (dock, _, _) = v.layout.find("fxbrowser").unwrap();
            let id = v.layout.docks[dock].id.clone();
            v.scroll_dock(&id, f32::MAX);
            cx.notify();
        });
        cx.run_until_parked();
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
        arrive(cx);
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
                            // Scrolling docks keep offscreen dividers in geometry.
                            if d.group.is_some()
                                && view.read_with(cx, |v, _| {
                                    v.geometry
                                        .side_docks
                                        .get(&d.dock)
                                        .is_some_and(|clip| !clip.contains(x, y))
                                })
                            {
                                continue;
                            }
                            // Where two dividers' grabs cross, either answers.
                            let crossing = dividers
                                .iter()
                                .enumerate()
                                .any(|(j, o)| j != i && grab(o).contains(x, y));
                            if crossing {
                                continue;
                            }
                            // Inside the panel's 11px corner, the area operation
                            // owns the press; the rest of the splitter grab is unchanged.
                            let corner = view.read_with(cx, |v, _| {
                                v.geometry.panels.iter().any(|p| {
                                    crate::area::Corner::ALL
                                        .into_iter()
                                        .any(|c| v.geometry.side_docks.get(&p.dock)
                                            .map_or(c.rect(p.rect), |clip| c.rect(p.rect).intersection(*clip))
                                            .contains(x, y))
                                })
                            });
                            cx.simulate_mouse_move(point(px(x), px(y)), None, none);
                            assert_eq!(
                                cx.cursor_style(),
                                if corner {
                                    CursorStyle::Crosshair
                                } else {
                                    wanted
                                },
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
                    // An offscreen splitter has no press target, just as it
                    // has no cursor target in the sweep above.
                    if d.group.is_some()
                        && view.read_with(cx, |v, _| {
                            v.geometry
                                .side_docks
                                .get(&d.dock)
                                .is_some_and(|clip| !clip.contains(x, y))
                        })
                    {
                        continue;
                    }
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
    // Panel dragging (DESIGN: what shows while dragging is what a release gives).
    fn drag_none() -> gpui::Modifiers {
        gpui::Modifiers::none()
    }
    fn at(p: (f32, f32)) -> gpui::Point<gpui::Pixels> {
        gpui::point(gpui::px(p.0), gpui::px(p.1))
    }
    /// The middle of `id`'s tab (or lone title).
    fn tab_middle(v: &Editor, id: &str) -> (f32, f32) {
        let r = v.tab_rect(id).unwrap_or_else(|| panic!("{id} has a tab"));
        (r.x + r.w / 2., r.y + r.h / 2.)
    }
    /// The rect of the stack `id` is in.
    fn stack_rect(v: &Editor, id: &str) -> crate::layout::Rect {
        let (d, g, _) = v.layout.find(id).unwrap();
        v.geometry
            .panels
            .iter()
            .find(|p| (p.dock, p.group) == (d, g))
            .unwrap()
            .rect
    }
    fn press(cx: &mut gpui::VisualTestContext, from: (f32, f32)) {
        cx.simulate_mouse_move(at(from), None, drag_none());
        cx.simulate_mouse_down(at(from), gpui::MouseButton::Left, drag_none());
    }
    fn pull(cx: &mut gpui::VisualTestContext, from: (f32, f32), to: (f32, f32)) {
        for s in 1..=8 {
            let t = s as f32 / 8.;
            let p = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            cx.simulate_mouse_move(at(p), gpui::MouseButton::Left, drag_none());
        }
    }
    /// The pointer rests where it is: the layout catches up with the target.
    fn rest(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<Editor>) {
        crate::drag::advance_frame_time(0.3);
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    }
    /// Time passes until every panel travelling has arrived.
    fn arrive(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<Editor>) {
        for _ in 0..4 {
            rest(cx, view);
        }
    }
    /// Where every panel is: each dock's stacks, each stack's tabs.
    fn places(layout: &crate::layout::Layout) -> Vec<Vec<Vec<String>>> {
        layout
            .docks
            .iter()
            .map(|d| d.groups.iter().map(|g| g.tabs.clone()).collect())
            .collect()
    }
    fn target(v: &Editor) -> Option<crate::layout::DropTarget> {
        match &v.gesture {
            Some(super::Gesture::Panel(d)) => d.preview.map(|p| p.target),
            _ => None,
        }
    }

    /// A press that stays within the threshold is a click: no drag, no
    /// chip, the layout as it was. Past it the panel is in the hand.
    #[gpui::test]
    fn a_click_never_starts_a_drag_and_five_points_do(cx: &mut TestAppContext) {
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        let from = view.read_with(cx, |v, _| tab_middle(v, "inspector"));
        let before = view.read_with(cx, |v, _| v.layout.clone());
        press(cx, from);
        cx.simulate_mouse_move(at((from.0 + 3., from.1 + 2.)), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_some(), "the press is held");
            assert!(!v.dragging_panel(), "3 pt is still a click");
            assert_eq!(v.layout, before);
        });
        cx.simulate_mouse_up(at((from.0 + 3., from.1 + 2.)), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            assert!(v.settle.is_none(), "a click leaves no chip");
            assert_eq!(v.layout, before);
        });
        press(cx, from);
        cx.simulate_mouse_move(at((from.0 + 6., from.1)), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            assert!(v.dragging_panel(), "6 pt lifts it");
            assert_eq!(v.layout, before, "over its own place nothing moves");
        });
        cx.simulate_mouse_up(at((from.0 + 6., from.1)), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| assert_eq!(v.layout, before));
    }

    /// Each kind of drop (a tab position, a column position beside a panel,
    /// a dock's edge): the window shows the dropped layout while the drag
    /// lasts, the release keeps exactly that, and it is written once (one
    /// undo step). A cancel, or a release back home, writes nothing.
    #[gpui::test]
    fn a_drag_shows_the_layout_its_drop_gives_and_lands_as_one_change(cx: &mut TestAppContext) {
        use crate::layout::DropTarget;
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        crate::drag::set_frame_time(Some(std::time::Instant::now()));
        let m = view.read_with(cx, |v, _| v.tokens.metrics);
        for kind in ["tab", "column", "dock"] {
            view.update(cx, |v, cx| {
                v.layout = crate::layout::Layout::preset("design");
                v.save();
                cx.notify();
            });
            cx.run_until_parked();
            let before = view.read_with(cx, |v, _| v.layout.clone());
            let written = view.read_with(cx, |v, _| v.outbox.len());
            let (id, to) = view.read_with(cx, |v, _| {
                let left = stack_rect(v, "assets");
                match kind {
                    // Onto the left stack's header, past its tabs: its last tab.
                    "tab" => ("easing", (left.x + left.w - 60., left.y + m.header / 2.)),
                    // Low in the left stack's body: below it in that column.
                    "column" => ("inspector", (left.x + left.w / 2., left.y + left.h * 0.92)),
                    // The window's bottom edge: into the bottom dock.
                    _ => (
                        "fxbrowser",
                        (
                            v.dock_bounds.x + v.dock_bounds.w / 2.,
                            v.dock_bounds.y + v.dock_bounds.h - 2.,
                        ),
                    ),
                }
            });
            let from = view.read_with(cx, |v, _| tab_middle(v, id));
            press(cx, from);
            pull(cx, from, to);
            // On the way the layout stands still; it re-flows once the
            // pointer rests on the target.
            // (A showing tab in the hand lets its stack show the next one.)
            assert_eq!(
                view.read_with(cx, |v, _| places(&v.layout)),
                places(&before),
                "{kind}: in passing"
            );
            rest(cx, &view);
            let (shown, aimed) = view.read_with(cx, |v, _| (v.layout.clone(), target(v)));
            assert_ne!(shown, before, "{kind}: the layout re-flows while dragging");
            match (kind, aimed) {
                ("tab", Some(DropTarget::Tab { .. }))
                | ("column" | "dock", Some(DropTarget::Split { .. })) => {}
                other => panic!("unexpected target: {other:?}"),
            }
            let left_dock = before.docks.iter().position(|d| d.id == "left").unwrap();
            let (dock, group, _) = shown.find(id).unwrap();
            match kind {
                "tab" => {
                    assert_eq!(dock, left_dock);
                    assert_eq!(shown.docks[dock].groups[group].tabs.last().unwrap(), id);
                    assert_eq!(shown.docks[dock].groups[group].active(), id);
                    // Its old stack shows the tab that was beside it.
                    let (d, g, _) = shown.find("fxbrowser").unwrap();
                    assert_eq!(shown.docks[d].groups[g].tabs, ["fxbrowser"]);
                }
                "column" => {
                    assert_eq!((dock, group), (left_dock, 1), "below the left stack");
                    assert_eq!(shown.docks[dock].groups[group].tabs, [id]);
                }
                _ => assert_eq!(shown.docks[dock].id, "bottom"),
            }
            assert_eq!(view.read_with(cx, |v, _| v.outbox.len()), written, "nothing saved yet");
            cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
            view.read_with(cx, |v, _| {
                assert!(v.gesture.is_none());
                assert_eq!(v.layout, shown, "{kind}: the release keeps what showed");
                assert_eq!(v.outbox.len(), written + 1, "{kind}: one change, one undo step");
                assert!(v.settle.is_some(), "the chip settles into place");
            });
            cx.run_until_parked();
        }
        // Escape (or the window letting go of the drag) puts everything back.
        view.update(cx, |v, cx| {
            v.layout = crate::layout::Layout::preset("design");
            v.save();
            cx.notify();
        });
        cx.run_until_parked();
        let before = view.read_with(cx, |v, _| v.layout.clone());
        let written = view.read_with(cx, |v, _| v.outbox.len());
        let from = view.read_with(cx, |v, _| tab_middle(v, "inspector"));
        let to = view.read_with(cx, |v, _| {
            let left = stack_rect(v, "assets");
            (left.x + left.w / 2., left.y + left.h * 0.92)
        });
        press(cx, from);
        pull(cx, from, to);
        rest(cx, &view);
        view.update(cx, |v, cx| {
            assert_ne!(v.layout, before);
            v.cancel_gesture(cx);
            assert_eq!(v.layout, before, "a cancel restores the layout");
            assert_eq!(v.outbox.len(), written, "and writes nothing");
            assert!(v.settle.as_ref().is_some_and(|s| s.slot.is_none()), "the chip goes home");
            assert!(v.slides.armed, "the panels travel back");
        });
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        cx.run_until_parked();
        // (The panel is back in its place before it is pressed again.)
        arrive(cx, &view);
        // Dragged out and brought back home: released there, nothing changes.
        press(cx, from);
        pull(cx, from, to);
        rest(cx, &view);
        assert_ne!(view.read_with(cx, |v, _| v.layout.clone()), before);
        pull(cx, to, (from.0 + 8., from.1));
        rest(cx, &view);
        view.read_with(cx, |v, _| {
            assert_eq!(target(v), None, "over its own place");
            assert_eq!(v.layout, before, "the layout shown is the original again");
        });
        cx.simulate_mouse_up(at((from.0 + 8., from.1)), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            assert_eq!(v.layout, before);
            assert_eq!(v.outbox.len(), written);
        });
        // Let go in passing, before the layout caught up: it still lands
        // on the target under the pointer, as one change.
        arrive(cx, &view);
        press(cx, from);
        pull(cx, from, to);
        let aimed = view.read_with(cx, |v, _| {
            assert_eq!(v.layout, before, "not shown yet");
            target(v)
        });
        assert!(matches!(aimed, Some(DropTarget::Split { .. })), "{aimed:?}");
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            assert_eq!(v.layout.find("inspector").map(|(d, g, _)| (d, g)), Some((0, 1)));
            assert_eq!(v.outbox.len(), written + 1);
        });
        crate::drag::set_frame_time(None);
    }

    /// Targets are read from the panels as they stood at pick-up and held
    /// with hysteresis: a pointer just over a boundary keeps the target it
    /// had, however the layout has re-flowed under it.
    #[gpui::test]
    fn a_drag_target_holds_at_a_boundary_while_the_layout_reflows(cx: &mut TestAppContext) {
        use crate::layout::DropTarget;
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        let m = view.read_with(cx, |v, _| v.tokens.metrics);
        let left = view.read_with(cx, |v, _| stack_rect(v, "assets"));
        // Where the left stack's body turns from "tab" to "below".
        let body = left.h - m.header;
        let boundary = left.y + m.header + body * (1. - crate::layout::BODY_EDGE);
        let x = left.x + left.w / 2.;
        let from = view.read_with(cx, |v, _| tab_middle(v, "inspector"));
        press(cx, from);
        pull(cx, from, (x, boundary - 30.));
        cx.run_until_parked();
        let tab = view.read_with(cx, |v, _| target(v));
        assert!(matches!(tab, Some(DropTarget::Tab { .. })), "{tab:?}");
        // The layout has re-flowed (the panel is a tab there now); just
        // over the line and back, again and again, nothing changes.
        for dy in [2., -2., 3., -1., 4., 0.] {
            cx.simulate_mouse_move(at((x, boundary + dy)), gpui::MouseButton::Left, drag_none());
            cx.run_until_parked();
            assert_eq!(view.read_with(cx, |v, _| target(v)), tab, "held at {dy}");
        }
        // Clear of the line: the neighbouring target, steadily.
        cx.simulate_mouse_move(at((x, boundary + 9.)), gpui::MouseButton::Left, drag_none());
        cx.run_until_parked();
        let below = view.read_with(cx, |v, _| target(v));
        assert!(matches!(below, Some(DropTarget::Split { .. })), "{below:?}");
        for dy in [-2., 1., -4.] {
            cx.simulate_mouse_move(at((x, boundary + dy)), gpui::MouseButton::Left, drag_none());
            cx.run_until_parked();
            assert_eq!(view.read_with(cx, |v, _| target(v)), below, "held at {dy}");
        }
        view.update(cx, |v, cx| v.cancel_gesture(cx));
    }

    /// A press while panels are still travelling lands on what is under
    /// the pointer: the header of a panel in mid-slide, pressed where it
    /// shows (not where it will rest), lifts that panel.
    #[gpui::test]
    fn a_press_on_a_sliding_panel_lifts_that_panel(cx: &mut TestAppContext) {
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        crate::drag::set_frame_time(Some(std::time::Instant::now()));
        let m = view.read_with(cx, |v, _| v.tokens.metrics);
        let home = view.read_with(cx, |v, _| stack_rect(v, "fxbrowser"));
        let from = view.read_with(cx, |v, _| tab_middle(v, "inspector"));
        // Properties dropped (in passing) below the Effects stack, in the
        // same column: the stack sets off upwards for the top of it.
        let to = (home.x + home.w / 2., home.y + home.h * 0.92);
        press(cx, from);
        pull(cx, from, to);
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        cx.run_until_parked();
        let place = view.read_with(cx, |v, _| stack_rect(v, "fxbrowser"));
        assert!(home.y - place.y > 100., "the stack has a new place above: {place:?}");
        // On its way, the header still more than its own height from rest.
        let mut shown = place;
        for _ in 0..40 {
            crate::drag::advance_frame_time(0.01);
            view.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            shown = view.read_with(cx, |v, _| v.slides.shown("fxbrowser", place));
            if shown.y - place.y <= 40. {
                break;
            }
        }
        assert!(shown.y - place.y > 18., "in mid-slide: {shown:?} on its way to {place:?}");
        // Its header where it shows now: its body, were it already at rest.
        let header = (shown.x + shown.w * 0.6, shown.y + m.header / 2.);
        assert!(header.1 > place.y + m.header, "not where the header will rest");
        press(cx, header);
        cx.simulate_mouse_move(at((header.0 + 8., header.1)), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| match &v.gesture {
            Some(super::Gesture::Panel(d)) => {
                assert!(
                    d.id == "fxbrowser" || d.id == "easing",
                    "the panel under the pointer lifts, not {}",
                    d.id
                );
                assert!(d.moved);
            }
            _ => panic!("the press on the sliding panel's header starts its drag"),
        });
        view.update(cx, |v, cx| v.cancel_gesture(cx));
        crate::drag::set_frame_time(None);
    }

    /// An extension's body is a native surface placed from a rect.
    struct NativeBody;
    impl crate::integration::Integration for NativeBody {
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
        fn native_panel(&self, id: &str, _: &App) -> bool {
            id == "inspector"
        }
        fn project_id(&self, _: &App) -> Option<String> {
            None
        }
        fn status(&self, _: &App) -> String {
            String::new()
        }
        fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    }

    /// With motion, panels travel to the previewed layout and an extension
    /// body's frame is its panel's animated rect on every frame, never its
    /// final place ahead of the panel. With Reduce Motion everything is
    /// placed at once and no chip glides, but the slot indicator still shows.
    #[gpui::test]
    fn native_bodies_follow_their_panel_and_reduce_motion_places_everything(
        cx: &mut TestAppContext,
    ) {
        for reduced in [false, true] {
            let tokens = Tokens::new(
                pm_design::Scheme::Dark,
                pm_design::Accessibility {
                    reduce_motion: reduced,
                    ..Default::default()
                },
            );
            let (view, cx) = cx.add_window_view(|_, cx| {
                let mut e = Editor::new(tokens, None, cx);
                e.integration = Some(Rc::new(NativeBody));
                e
            });
            cx.run_until_parked();
            crate::drag::set_frame_time(Some(std::time::Instant::now()));
            let m = view.read_with(cx, |v, _| v.tokens.metrics);
            let edge = m.border.max(m.splitter_reach);
            let body = |v: &Editor| {
                v.native_body_rects
                    .iter()
                    .find(|(id, _)| id == "inspector")
                    .map(|(_, r)| *r)
                    .expect("the extension body is placed")
            };
            let home = view.read_with(cx, |v, _| stack_rect(v, "inspector"));
            assert_eq!(
                view.read_with(cx, |v, _| body(v)),
                super::native_body(home, edge, m.header),
                "at rest: inside its panel"
            );
            let from = view.read_with(cx, |v, _| tab_middle(v, "inspector"));
            let to = view.read_with(cx, |v, _| {
                let left = stack_rect(v, "assets");
                (left.x + left.w / 2., left.y + left.h * 0.92)
            });
            press(cx, from);
            pull(cx, from, to);
            if !reduced {
                // (Reduce Motion shows each target at once.)
                assert_eq!(view.read_with(cx, |v, _| stack_rect(v, "inspector")), home);
                crate::drag::advance_frame_time(0.3);
                // The frame the layout catches up on: nothing has moved yet.
                view.update(cx, |v, _| v.retarget(false));
            }
            view.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            view.read_with(cx, |v, _| {
                let place = stack_rect(v, "inspector");
                assert_ne!(place, home, "the preview moved it");
                let shown = v.slides.shown("inspector", place);
                assert_eq!(body(v), super::native_body(shown, edge, m.header));
                assert!(v.slot_rect("inspector").is_some(), "the slot indicator shows");
                if reduced {
                    assert_eq!(shown, place, "placed at once");
                    assert!(v.slides.moving.is_empty() && !v.tab_slides.travelling());
                } else {
                    assert!(
                        (shown.x - place.x).abs() > 100.,
                        "still on its way from the right: {shown:?} to {place:?}"
                    );
                }
            });
            // Part of the way, then there: the body is with the panel throughout.
            for (step, seconds) in [0.05, 0.25, 0.25, 0.25].into_iter().enumerate() {
                crate::drag::advance_frame_time(seconds);
                view.update(cx, |_, cx| cx.notify());
                cx.run_until_parked();
                view.read_with(cx, |v, _| {
                    let place = stack_rect(v, "inspector");
                    let shown = v.slides.shown("inspector", place);
                    assert_eq!(body(v), super::native_body(shown, edge, m.header));
                    if step == 3 {
                        assert_eq!(shown, place, "arrived");
                    }
                });
            }
            cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
            view.read_with(cx, |v, _| {
                assert_eq!(v.settle.is_none(), reduced, "no glide with Reduce Motion");
                assert!(v.layout.find("inspector").is_some());
            });
            crate::drag::set_frame_time(None);
        }
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

#[cfg(test)]
mod dock_tab_capture_tests {
    use super::*;
    use std::sync::Arc;
    #[::core::prelude::v1::test]
    fn dock_tabs_capture() {
        let Ok(dir) = std::env::var("PM_DOCK_CAPTURE") else {
            return;
        };
        objc2::rc::autoreleasepool(|_| {
            let mut cx = HeadlessAppContext::with_platform(
                Arc::new(gpui_parley::ParleyTextSystem::new(
                    gpui_parley::SystemFonts::Load,
                )),
                Arc::new(()),
                || {
                    Some(Box::new(
                        gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                    ))
                },
            );
            let t = Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default());
            let window = cx
                .open_window(size(px(1100.), px(700.)), |_, cx| {
                    cx.new(|cx| {
                        let mut editor = Editor::new(t, None, cx);
                        editor.layout = Layout::preset("design");
                        editor.layout.move_panel("fxbrowser", 0, 0, true).unwrap();
                        editor
                    })
                })
                .unwrap();
            cx.run_until_parked();
            std::fs::create_dir_all(&dir).unwrap();
            cx.capture_screenshot(window.into())
                .unwrap()
                .save(format!("{dir}/dock-tabs.png"))
                .unwrap();
            if std::env::var_os("PM_DOCK_BASELINE").is_none() {
                fn assert_strips(v: &Editor) {
                    for p in &v.geometry.panels {
                        let group = &v.layout.docks[p.dock].groups[p.group];
                        assert_eq!(v.geometry.tabs[&(p.dock, p.group)].len(), group.tabs.len());
                    }
                }
                let restored = window.update(&mut cx, |v, _, _| {
                    assert_strips(v);
                    serde_json::to_string(&v.layout).unwrap()
                }).unwrap();
                window.update(&mut cx, |v, _, cx| {
                    assert!(v.run_shell_command("panel:close:fxbrowser", cx));
                }).unwrap();
                cx.run_until_parked();
                window.update(&mut cx, |v, _, cx| {
                    assert_strips(v);
                    v.layout = serde_json::from_str(&restored).unwrap();
                    cx.notify();
                }).unwrap();
                cx.run_until_parked();
                window.update(&mut cx, |v, _, cx| {
                    assert_strips(v);
                    assert!(v.run_shell_command("panel:activate:layer-effects", cx));
                }).unwrap();
                cx.run_until_parked();
                window.update(&mut cx, |v, _, _| assert_strips(v)).unwrap();
            }
        });
    }
}

#[cfg(all(test, target_os = "macos"))]
mod library_capture_tests {
    use super::*;
    use gpui::{AppContext, HeadlessAppContext, size};
    use std::sync::Arc;

    #[test]
    #[ignore = "writes headless Library reveal proof; set PM_DOCK_CAPTURE_DIR"]
    fn capture_library_reveal() {
        let dir = std::env::var("PM_DOCK_CAPTURE_DIR").unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        for (name, scheme) in [
            ("light", pm_design::Scheme::Light),
            ("dark", pm_design::Scheme::Dark),
        ] {
            for reduced in [false, true] {
                objc2::rc::autoreleasepool(|_| {
                    let mut cx = HeadlessAppContext::with_platform(
                        Arc::new(gpui_parley::ParleyTextSystem::new(
                            gpui_parley::SystemFonts::Load,
                        )),
                        Arc::new(()),
                        || {
                            Some(Box::new(
                                gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                            ))
                        },
                    );
                    let tokens = Tokens::new(
                        scheme,
                        pm_design::Accessibility {
                            reduce_motion: reduced,
                            ..Default::default()
                        },
                    );
                    let window = cx
                        .open_window(size(px(1000.), px(600.)), |_, cx| {
                            cx.new(|cx| {
                                let mut v = Editor::new(tokens, None, cx);
                                let right =
                                    v.layout.docks.iter_mut().find(|d| d.id == "right").unwrap();
                                right.groups = ["inspector", "easing", "fxbrowser"]
                                    .into_iter()
                                    .map(|id| layout::Group {
                                        tabs: vec![id.into()],
                                        active: 0,
                                        size: Some(400.),
                                        collapsed: false,
                                    })
                                    .collect();
                                v
                            })
                        })
                        .unwrap();
                    cx.run_until_parked();
                    let mode = if reduced { "reduced" } else { "motion" };
                    cx.capture_screenshot(window.into())
                        .unwrap()
                        .save(format!("{dir}/{name}-library-{mode}-before.png"))
                        .unwrap();
                    window
                        .update(&mut cx, |v, _, cx| {
                            v.show_library_panel("fxbrowser", cx).unwrap();
                        })
                        .unwrap();
                    cx.update_window(window.into(), |_, w, cx| {
                        w.draw(cx).clear(cx);
                    })
                    .unwrap();
                    window
                        .update(&mut cx, |v, _, cx| {
                            // Sample the animation clocks deterministically, without OS timers.
                            if let Some((_, _, _, start)) = &mut v.reveal_motion {
                                *start =
                                    std::time::Instant::now() - std::time::Duration::from_secs(1);
                            }
                            if !reduced {
                                v.header_wash =
                                    Some(("fxbrowser".into(), std::time::Instant::now()));
                            }
                            cx.notify();
                        })
                        .unwrap();
                    cx.update_window(window.into(), |_, w, cx| {
                        w.draw(cx).clear(cx);
                    })
                    .unwrap();
                    window
                        .update(&mut cx, |v, _, _| {
                            let r = v.panel_rect("fxbrowser").unwrap();
                            assert!(r.y >= 0. && r.y + r.h <= 600., "revealed: {r:?}");
                            assert_eq!(v.header_wash.is_none(), reduced);
                        })
                        .unwrap();
                    cx.capture_screenshot(window.into())
                        .unwrap()
                        .save(format!("{dir}/{name}-library-{mode}-shown.png"))
                        .unwrap();
                    if !reduced {
                        window
                            .update(&mut cx, |v, _, cx| {
                                v.header_wash = Some((
                                    "fxbrowser".into(),
                                    std::time::Instant::now() - std::time::Duration::from_secs(2),
                                ));
                                cx.notify();
                            })
                            .unwrap();
                        cx.update_window(window.into(), |_, w, cx| {
                            w.draw(cx).clear(cx);
                        })
                        .unwrap();
                        cx.capture_screenshot(window.into())
                            .unwrap()
                            .save(format!("{dir}/{name}-library-settled.png"))
                            .unwrap();
                    }
                });
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "dock_render_tests.rs"]
mod dock_render_tests;
