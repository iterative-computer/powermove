use crate::kit::Recipes as _;
use crate::{
    layout::{self, Divider, DropPreview, DropTarget, Edge, Geometry, Layout, Rect},
    native::{Command, Host, MenuEntry, MenuState, NativePanelGeometry},
    session::WindowState,
    workspace,
};
#[cfg(test)]
use ::core::prelude::v1::test;
use gpui::{prelude::*, *};
use pm_design::Tokens;
use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    io::Write as _,
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};
use unicode_segmentation::UnicodeSegmentation;

#[path = "still_drag.rs"]
mod still_drag;

/// The frozen input zones and the one prospective destination.
#[derive(Clone)]
struct PanelDrag {
    id: String,
    stack: bool,
    start: Point<Pixels>,
    original: Layout,
    original_maximized: Option<String>,
    at: Point<Pixels>,
    moved: bool,
    preview: Option<DropPreview>,
    aim: Option<DropPreview>,
    chip: Option<still_drag::Chip>,
    /// False for a panel dragged in from outside.
    live: bool,
    geometry: Geometry,
    /// How far the drag's own edge scroll has moved each column since lift.
    scrolled: HashMap<String, f32>,
    lifted: bool,
    /// When the edge scroll last ran.
    scroll_tick: Option<std::time::Instant>,
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

fn panel_history_direction(command: &str) -> Option<bool> {
    match command {
        "undo" | "contextUndo" => Some(false),
        "redo" | "contextRedo" => Some(true),
        _ => None,
    }
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
    scene: Entity<WorkspaceView>,
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
    /// Where each extension body was placed this frame (its panel's
    /// animated rect, less the header and the dividers' grab).
    native_body_rects: Vec<(String, Rect)>,
    /// The aperture clip each native body was published with (the dock
    /// viewport and the travelling sticky header). Test mirror of
    /// `Host::native_panel_clips` for hosts that cannot exist headless.
    #[cfg(test)]
    native_clip_rects: Vec<(String, Rect)>,
    #[cfg(test)]
    native_ground_holes: Rc<RefCell<Vec<Rect>>>,
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
    dismissed_panels: BTreeSet<String>,
    /// Docking area of the last render, in window points.
    dock_bounds: Rect,
    gesture: Option<Gesture>,
    cancelled_press: bool,
    panel_undo: Vec<(Layout, Layout)>,
    panel_redo: Vec<(Layout, Layout)>,
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
    resize_geometry: Option<Geometry>,
    tab_scrolls: HashMap<String, TabScroll>,
    /// Hover washes of the panel tabs.
    motion: crate::motion::Motion<u64>,
    header_tools_widths: Rc<RefCell<HashMap<String, f32>>>,
    extension_slot_heights: Rc<RefCell<HashMap<String, f32>>>,
    hidden_panel_headers: std::collections::HashSet<String>,
    #[cfg(test)]
    rendered_tab_labels: HashMap<(usize, usize), Vec<RenderedTabLabel>>,
    deferred_panel_commands: Vec<String>,
}

/// A scrolling tab strip's offset, in points: it follows the trackpad
/// continuously, as any macOS strip does, and a newly selected tab is scrolled
/// just into view.
#[derive(Default)]
struct TabScroll {
    selected: String,
    viewport: f32,
    active: usize,
    offset: f32,
}

#[cfg(test)]
struct RenderedTabLabel {
    left: f32,
    right: f32,
    font_size: f32,
    tab_width: f32,
    floor: f32,
}

impl TabScroll {
    /// Scrolls the active tab fully into view when the selection, the
    /// viewport or the active index changes; otherwise keeps the offset.
    fn reveal(&mut self, selected: &str, active: usize, viewport: f32, widths: &[f32], max: f32) {
        if self.selected != selected || self.viewport != viewport || self.active != active {
            self.selected = selected.to_owned();
            self.viewport = viewport;
            self.active = active;
            let start = widths[..active].iter().sum::<f32>() + active as f32 * TAB_SPACING;
            let end = start + widths.get(active).copied().unwrap_or(0.);
            if start < self.offset {
                self.offset = start;
            } else if end > self.offset + viewport {
                self.offset = end - viewport;
            }
        }
        self.offset = self.offset.clamp(0., max.max(0.));
    }

    /// Moves by a scroll delta in points; true when the offset changed.
    fn scroll(&mut self, delta: f32, max: f32) -> bool {
        let before = self.offset;
        self.offset = (self.offset - delta).clamp(0., max.max(0.));
        self.offset != before
    }
}

struct WorkspaceView(WeakEntity<Editor>);

impl Render for WorkspaceView {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.0.update(cx, |editor, cx| editor.render_workspace(w, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
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
        let editor = cx.weak_entity();
        Self {
            scene: cx.new(|_| WorkspaceView(editor)),
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
            native_body_rects: Vec::new(),
            #[cfg(test)]
            native_clip_rects: Vec::new(),
            #[cfg(test)]
            native_ground_holes: Rc::default(),
            maximized_panel: None,
            parked: HashMap::new(),
            awaiting_panels: true,
            absent: Default::default(),
            pending_restores: Vec::new(),
            dismissed_panels: workspace::dismissed_panels(&record),
            dock_bounds: Rect::default(),
            gesture: None,
            cancelled_press: false,
            panel_undo: Vec::new(),
            panel_redo: Vec::new(),
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
            resize_geometry: None,
            tab_scrolls: HashMap::new(),
            motion: Default::default(),
            header_tools_widths: Rc::default(),
            extension_slot_heights: Rc::default(),
            hidden_panel_headers: Default::default(),
            #[cfg(test)]
            rendered_tab_labels: HashMap::new(),
            deferred_panel_commands: Vec::new(),
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
        if let Gesture::Panel(drag) = g {
            if drag.live {
                self.maximized_panel = drag.original_maximized;
            }
        } else {
            self.layout = g.original();
        }
        self.resize_live = false;
        self.resize_geometry = None;
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
        self.pending_restores.clear();
        self.panel_undo.clear();
        self.panel_redo.clear();
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
        if self.dragging_panel() {
            cx.stop_propagation();
            return;
        }
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

    fn dock_fade(&self, dock: usize) -> (f32, f32) {
        let id = &self.layout.docks[dock].id;
        let offset = self.dock_scroll.get(id).copied().unwrap_or(0.);
        let maximum = self.dock_scroll_max.get(id).copied().unwrap_or(0.);
        let depth = pm_design::DOCK_SCROLL_FADE;
        // The fade grows with what has gone past the edge: a column a point
        // from its end fades a point, never the whole ramp at once (the top
        // header blinked between crisp and faded as a scroll crossed 0).
        (offset.clamp(0., depth), (maximum - offset).clamp(0., depth))
    }

    fn dock_body_fade(&self, dock: usize, clip: Rect) -> [f64; 2] {
        let Some(viewport) = self.geometry.side_docks.get(&dock) else {
            return [0., 0.];
        };
        let (top, bottom) = self.dock_fade(dock);
        [
            f64::from((top - (clip.y - viewport.y).max(0.)).max(0.)),
            f64::from((bottom - (viewport.y + viewport.h - clip.y - clip.h).max(0.)).max(0.)),
        ]
    }

    /// The column's fade on a panel in it. A panel whose header is pinned
    /// at the column's top (`pinned`) takes none there: the header is the
    /// edge, drawn whole, and the body scrolls under it.
    fn dock_fade_edges(&self, dock: usize, pinned: bool) -> Edges<AbsoluteLength> {
        let (top, bottom) = self.dock_fade(dock);
        let top = if pinned { 0. } else { top };
        Edges {
            top: px(top).into(),
            bottom: px(bottom).into(),
            left: px(0.).into(),
            right: px(0.).into(),
        }
    }

    /// Save the layout in its workspace record and session.
    pub fn save(&mut self) {
        self.dismissed_panels.retain(|id| self.layout.find(id).is_none());
        let after = self.workspace_with_layout(&self.layout);
        if after != self.record {
            self.commit(after);
        }
        self.persist_session();
    }

    fn workspace_with_layout(&self, layout: &Layout) -> serde_json::Value {
        // A parked panel keeps its place in the workspace (v1 kept a retired
        // panel's spec); only this window's view leaves it out.
        let mut written = layout.clone();
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
        let mut record = workspace::write(&written, &self.record);
        workspace::write_dismissed_panels(&mut record, &self.dismissed_panels);
        record
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
        self.panel_undo.clear();
        self.panel_redo.clear();
        // Places in the layout this replaces; the new one's absent panels
        // are found again on the next frame.
        for id in std::mem::take(&mut self.absent) {
            self.parked.remove(&id);
        }
        self.layout = workspace::layout(&record);
        self.dismissed_panels = workspace::dismissed_panels(&record);
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
        // The divider owns its draft until mouse-up. An async publication is
        // reconciled with the final width then, not painted over the draft.
        if matches!(self.gesture, Some(Gesture::Resize(..))) || self.dragging_panel() {
            return;
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
    fn open_panels(&self) -> Vec<&str> {
        self.layout
            .open_panels()
            .into_iter()
            .filter(|id| !self.absent.contains(*id))
            .collect()
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
            workspace_history: [false; 2],
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
    fn menu_state_with_focus(&self, window: &Window, cx: &App) -> MenuState {
        let mut state = self.menu_state(cx);
        if self.shell_history_focus(window, cx) {
            state.workspace_history = [
                self.can_restore_panel_drop(false),
                self.can_restore_panel_drop(true),
            ];
        }
        state
    }
    fn dock_visible(&self, id: &str) -> bool {
        self.layout.docks.iter().any(|d| {
            d.id == id && !d.collapsed
                && d.groups.iter().any(|g| g.tabs.iter().any(|id| !self.absent.contains(id)))
        })
    }

    fn visible_geometry(&self) -> Geometry {
        self.layout.visible_geometry(
            self.dock_bounds,
            &self.tokens,
            &self.dock_natural_heights,
            &self.absent,
            self.maximized_panel.as_deref(),
        )
    }
    /// After discovery, unavailable panels keep their saved slots but don't
    /// occupy an otherwise empty stack or count as shown in the Window menu.
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
        let mut parts = name.splitn(3, ':');
        let (Some(kind), Some(verb)) = (parts.next(), parts.next()) else {
            return false;
        };
        let rest = parts.next().unwrap_or("");
        // No project (Home after the last project closed): there is no layout
        // to save. The same session state disables the library's button; Home
        // over an open project still saves that project's layout.
        if (kind, verb) == ("layout", "save")
            && self
                .integration
                .as_ref()
                .is_some_and(|i| i.project_id(cx).is_none())
        {
            return true;
        }
        if (self.dragging_panel() || matches!(self.gesture, Some(Gesture::Resize(..))))
            && matches!((kind, verb),
                ("panel", "registered" | "show" | "restore" | "park")
                | ("panels", "waiting" | "ready"))
        {
            self.deferred_panel_commands.push(name.to_owned());
            return true;
        }
        if (kind, verb) == ("panel", "restore") && self.dismissed_panels.contains(rest) {
            return true;
        }
        // A command that rearranges the docks moves panels to their new places.
        self.slides.armed = true;
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
            && matches!(verb, "close" | "toggle" | "remove")
            && let Some((dock, group, index)) = self.layout.find(rest)
        {
            let place = if self.layout.docks[dock].groups[group].tabs.len() == 1 {
                DropTarget::Split { dock, group }
            } else {
                DropTarget::Tab { dock, group, index }
            };
            self.library_places.insert(rest.into(), place);
        }
        if kind == "panel" && matches!(verb, "toggle" | "show" | "activate") {
            crate::telemetry::panel_requested(rest);
        }
        let result = match (kind, verb) {
            ("panel", "toggle") => {
                if self.layout.open_panels().contains(&rest) && layout::closable(rest) {
                    return self.shell_command(&format!("panel:close:{rest}"), cx);
                } else {
                    if let Err(error) = self.show_library_panel(rest, cx) {
                        self.status = error;
                    }
                    cx.notify();
                    return true;
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
                    self.maximized_panel = None;
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
                if rest != "agent" {
                    self.parked.remove(rest);
                    self.absent.remove(rest);
                }
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
                    self.parked.remove(rest);
                    self.layout.close_panel(rest)
                } else {
                    Ok(())
                }
            }
            ("panel", "close") => {
                // The frame that shows it closed: its feel line (`PM_FEEL_TRACE`).
                gpui_macos::feel::note_now("panel.close");
                self.layout.close_panel(rest).map(|_| {
                    self.parked.remove(rest);
                    if !layout::PANELS.contains(&rest) {
                        self.dismissed_panels.insert(rest.to_owned());
                    }
                })
            }
            // panel:remove:<id> — its extension went away (disabled,
            // uninstalled, rebuilt without it): closed, and whether the
            // person had dismissed it stays as it was, so re-enabling or
            // reinstalling brings it back unless they had closed it.
            ("panel", "remove") => self.layout.close_panel(rest).map(|_| {
                self.parked.remove(rest);
            }),
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
                        if t != rest && layout::closable(&t) {
                            self.parked.remove(&t);
                            if !layout::PANELS.contains(&t.as_str()) {
                                self.dismissed_panels.insert(t);
                            }
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
                    self.layout
                        .placed(p)
                        .is_some_and(|g| g.tabs.iter().any(|tab| tab == id))
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
                    .find(|p| self.layout.placed(p).is_some_and(|g| g.active() == *panel))
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
        if let Some(redo) = panel_history_direction(&cmd.name)
            && self.shell_history_focus(window, cx)
            && self.restore_panel_drop(redo)
        {
            if let (Some(token), Some(host)) = (cmd.token, &self.host) {
                host.borrow_mut().resolve(token, true);
            }
            cx.notify();
            return;
        }
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
                    let state = self.menu_state_with_focus(window, cx);
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
                        .and_then(|p| self.layout.placed(p).map(|g| g.active().to_owned()))
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
            // AppKit may move the full-screen toolbar without a window resize.
            "shellFullscreenChanged" => {}
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
        self.retarget(false);
    }
    /// A drag of `id` (its whole stack when `stack`) beginning at `at`, with
    /// the layout, the geometry and the column scroll as they are now.
    fn new_drag(&self, id: &str, at: Point<Pixels>, stack: bool, live: bool) -> PanelDrag {
        PanelDrag {
            id: id.into(),
            stack,
            start: at,
            original: self.layout.clone(),
            original_maximized: self.maximized_panel.clone(),
            at,
            moved: false,
            preview: None,
            aim: None,
            chip: None,
            live,
            geometry: self.geometry.clone(),
            scrolled: HashMap::new(),
            lifted: false,
            scroll_tick: None,
        }
    }
    /// The zones a drag's targets are read from: the panels as drawn at
    /// pick-up, moved only by the drag's own edge scroll.
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
    /// Hit-test cheaply per pointer move. Solve the prospective layout only
    /// when the destination changes, never to display that layout itself.
    fn retarget(&mut self, scrolled: bool) {
        let Some(Gesture::Panel(drag)) = &mut self.gesture else {
            return;
        };
        if !drag.live || !drag.moved {
            return;
        }
        if !drag.lifted {
            drag.lifted = true;
            self.slides.snap();
            self.tab_slides.snap();
            if drag.original == self.layout {
                drag.geometry = self.geometry.clone();
            }
            self.reveal_motion = None;
            self.reveal_panel = None;
        }
        let zones = Self::drag_zones(drag);
        let (bounds, tokens) = (self.dock_bounds, &self.tokens);
        let at = (f32::from(drag.at.x), f32::from(drag.at.y));
        let aim = crate::drag::steady(drag.aim, at, |at| {
            drag.original
                .drag_zone(&zones, bounds, at, tokens, &drag.id, drag.stack)
                .ok()
        });
        let changed = aim.map(|p| p.target) != drag.aim.map(|p| p.target);
        if !changed && !scrolled {
            return;
        }
        drag.aim = aim;
        drag.preview = aim.and_then(|mut p| {
            let mut next = drag.original.clone();
            next.apply_drop(&drag.id, &p, drag.stack).ok()?;
            if next == drag.original {
                return None;
            }
            if let Some(line) = still_drag::tab_line(p.target, &zones, tokens) {
                p.rect = line;
            } else {
                let (dock, group, _) = next.find(&drag.id)?;
                let layout = next.visible_geometry(bounds, tokens, &self.dock_natural_heights, &self.absent, None);
                p.rect = layout.panels.iter().find(|q| (q.dock, q.group) == (dock, group))?.rect;
                if let Some(viewport) = layout.side_docks.get(&dock) {
                    let bottom = layout.panels.iter().filter(|q| q.dock == dock)
                        .map(|q| q.rect.y + q.rect.h).fold(viewport.y, f32::max);
                    let max = (bottom - viewport.y - viewport.h).max(0.);
                    let offset = self.dock_scroll.get(&next.docks[dock].id).copied().unwrap_or(0.);
                    p.rect.y -= if max <= 1. { 0. } else { offset.min(max) };
                    p.rect = p.rect.intersection(*viewport);
                }
            }
            Some(p)
        });

    }
    /// While a drag is held near the top or bottom of a scrolling column,
    /// the column scrolls towards that edge and the target follows.
    fn drag_edge_scroll(&mut self, window: &mut Window) -> bool {
        let Some(Gesture::Panel(drag)) = &mut self.gesture else {
            return false;
        };
        if !drag.live || !drag.moved {
            return false;
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
            return false;
        };
        let now = crate::drag::now();
        let dt = drag.scroll_tick.replace(now).map_or(0., |last| {
            now.saturating_duration_since(last).as_secs_f32().min(0.05)
        });
        window.request_animation_frame();
        let next = (old + speed * dt).clamp(0., most);
        if next == old {
            return false;
        }
        *drag.scrolled.entry(id.clone()).or_default() += next - old;
        self.dock_scroll.insert(id, next);
        self.retarget(true);
        true
    }
    /// Where tab `id` (or a lone title) is drawn in its header, as last
    /// rendered. For headless recordings and tests.
    #[doc(hidden)]
    /// How far dock `id` (a column of panels) is scrolled (benches).
    #[doc(hidden)]
    pub fn dock_scroll_offset(&self, id: &str) -> f32 {
        self.dock_scroll.get(id).copied().unwrap_or(0.)
    }
    /// The dividers as last laid out (benches drive their drags).
    #[doc(hidden)]
    pub fn dividers(&self) -> &[layout::Divider] {
        &self.geometry.dividers
    }
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

    /// Keep unwinding inside the editor's GPUI update. A panic escaping the
    /// AppKit mouse callback aborts the process.
    fn guard_panel_drag<T>(
        &mut self,
        cx: &mut Context<Self>,
        operation: impl FnOnce(&mut Self, &mut Context<Self>) -> T,
    ) -> Option<T> {
        match catch_unwind(AssertUnwindSafe(|| operation(self, cx))) {
            Ok(result) => Some(result),
            Err(panic) => {
                let message = panic
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown panic");
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "Dock panel move cancelled: {message}"
                );
                self.gesture = None;
                self.slides.snap();
                self.tab_slides.snap();
                self.notice = None;
                self.status = "Panel move cancelled".into();
                cx.notify();
                None
            }
        }
    }

    /// A panel that is not in the workspace (the Agent popover, a window of
    /// its own) dragged over it to `at` (this window's points): the same drop
    /// preview a tab drag shows, for the panel as Pin would first place it.
    /// True when a drop there would dock it.
    pub fn drag_from_outside(&mut self, id: &str, at: (f32, f32), cx: &mut Context<Self>) -> bool {
        self.guard_panel_drag(cx, |editor, cx| editor.preview_outside_drag(id, at, cx))
            .unwrap_or(false)
    }

    fn preview_outside_drag(&mut self, id: &str, at: (f32, f32), cx: &mut Context<Self>) -> bool {
        if let Some(Gesture::Panel(drag)) = &mut self.gesture
            && !drag.live
            && drag.original != self.layout
        {
            drag.preview = None;
            cx.notify();
            return false;
        }
        let preview = self.outside_target(id, at, cx);
        let pointer = point(px(at.0), px(at.1));
        let mut drag = match self.gesture.take() {
            Some(Gesture::Panel(d)) if !d.live => d,
            Some(Gesture::Panel(_)) | None => self.new_drag(id, pointer, false, false),
            Some(g) => {
                self.layout = g.original();
                self.new_drag(id, pointer, false, false)
            }
        };
        if drag.id != id {
            drag.id = id.into();
            drag.chip = None;
        }
        drag.at = pointer;
        drag.moved = true;
        drag.lifted = true;
        self.slides.snap();
        self.tab_slides.snap();
        drag.preview = preview;
        self.gesture = Some(Gesture::Panel(drag));
        cx.notify();
        preview.is_some()
    }
    /// Ends an outside drag as a drop: the panel docks where its preview
    /// showed (the workspace changes as a Pin there would). False, with the
    /// workspace untouched, when no target was lit.
    pub fn drop_from_outside(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        if !matches!(&self.gesture, Some(Gesture::Panel(d)) if !d.live && d.id == id) {
            return false;
        }
        self.guard_panel_drag(cx, |editor, cx| editor.finish_outside_drop(id, cx))
            .unwrap_or(false)
    }

    fn finish_outside_drop(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(Gesture::Panel(drag)) = self.gesture.take() else {
            return false;
        };
        cx.notify();
        let Some(preview) = drag.preview.filter(|_| drag.original == self.layout) else {
            return false;
        };
        match self
            .layout
            .plan_floating_drop(id, preview.target, &self.panels(cx))
        {
            Ok(next) => {
                self.commit_panel_drop(next);
                true
            }
            Err(error) => {
                self.status = error.to_string();
                false
            }
        }
    }
    /// An outside drag let go off any target, or cancelled: no preview, the
    /// workspace as it was.
    pub fn cancel_from_outside(&mut self, cx: &mut Context<Self>) {
        if matches!(&self.gesture, Some(Gesture::Panel(d)) if !d.live) {
            self.gesture = None;
            cx.notify();
        }
    }
    /// Hit-test the workspace as drawn, then check that the floating panel
    /// can be inserted at the displayed target.
    fn outside_target(&self, id: &str, at: (f32, f32), cx: &App) -> Option<DropPreview> {
        // A floating panel's own drag docks as the original app's did: near
        // a side column's edge or between its panels; over the middle of
        // the workspace it stays floating (`Layout::floating_drop_target`).
        let preview = self.layout.floating_drop_target(
            &self.geometry,
            self.dock_bounds,
            at.0,
            at.1,
            &self.tokens,
        )?;
        self.layout
            .plan_floating_drop(id, preview.target, &self.panels(cx))
            .ok()?;
        Some(preview)
    }
    /// Completes a simulated drag like a mouse-up; returns the applied target.
    #[doc(hidden)]
    pub fn simulate_drop(&mut self) -> Option<DropTarget> {
        let Some(Gesture::Panel(drag)) = self.gesture.take() else {
            return None;
        };
        let preview = drag.preview.filter(|_| drag.original == self.layout)?;
        let mut next = drag.original;
        next.apply_drop(&drag.id, &preview, drag.stack).ok()?;
        self.commit_panel_drop(next);
        Some(preview.target)
    }
    fn commit_panel_drop(&mut self, next: Layout) {
        if next == self.layout {
            return;
        }
        let after = self.workspace_with_layout(&next);
        // Docking keeps the workspace ID. Read the native session state
        // before changing the layout, history or queued workspace writes.
        self.persist_session();
        if after != self.record {
            self.commit(after);
        }
        let before = std::mem::replace(&mut self.layout, next);
        // Do not undo through a workspace edit made outside this history.
        if self
            .panel_undo
            .last()
            .is_some_and(|(_, after)| *after != before)
        {
            self.panel_undo.clear();
        }
        self.panel_redo.clear();
        if self.panel_undo.len() == 32 {
            self.panel_undo.remove(0);
        }
        self.panel_undo.push((before, self.layout.clone()));
        self.slides.snap();
        self.tab_slides.snap();
    }

    fn shell_history_focus(&self, window: &Window, cx: &App) -> bool {
        window.focused(cx).is_none_or(|f| f == self.focus || f == self.workspace_focus)
            && self.host.as_ref().is_none_or(|h| h.borrow().shell_owns_history())
    }
    fn can_restore_panel_drop(&self, redo: bool) -> bool {
        if self.gesture.is_some() {
            return false;
        }
        if redo {
            self.panel_redo.last().is_some_and(|(before, _)| *before == self.layout)
        } else {
            self.panel_undo.last().is_some_and(|(_, after)| *after == self.layout)
        }
    }
    fn restore_panel_drop(&mut self, redo: bool) -> bool {
        if !self.can_restore_panel_drop(redo) {
            return false;
        }
        let (from, to) = if redo {
            (&mut self.panel_redo, &mut self.panel_undo)
        } else {
            (&mut self.panel_undo, &mut self.panel_redo)
        };
        let Some((before, after)) = from.pop() else {
            return false;
        };
        self.layout = if redo { after.clone() } else { before.clone() };
        to.push((before, after));
        self.slides.snap();
        self.tab_slides.snap();
        self.save();
        true
    }
    /// Ends the drag in progress as a release does: a panel moved lands
    /// where its preview showed, a resize keeps the sizes it reached.
    fn end_gesture(&mut self, at: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        if let Some(Gesture::Panel(drag)) = &self.gesture {
            // The floating window owns release of its own drag.
            if !drag.live {
                return;
            }
            if drag.moved {
                self.guard_panel_drag(cx, |editor, cx| editor.finish_panel_drop(at, cx));
                return;
            }
        }
        let Some(g) = self.gesture.take() else {
            return;
        };
        let resized = std::mem::take(&mut self.resize_live);
        self.resize_geometry = None;
        if let Gesture::Resize(d, _, original) = &g {
            if resized && self.layout != *original {
                if !d.horizontal {
                    let draft = self.layout.clone();
                    self.sync_workspaces(cx);
                    if self.layout.workspace == draft.workspace {
                        let column = &draft.docks[d.dock];
                        if let Some(dock) = self.layout.docks.iter_mut().find(|d| d.id == column.id) {
                            dock.size = column.size.map(f32::round);
                        }
                    }
                }
                self.save();
            }
            cx.notify();
            return;
        }
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
        if let Gesture::Panel(drag) = g {
            if !drag.stack && let Some((d, g, i)) = self.layout.find(&drag.id) {
                self.layout.docks[d].groups[g].active = i;
                self.save();
            }
            cx.notify();
            return;
        }
        self.slides.armed = true;
        self.tab_slides.armed = true;
        self.save();
        cx.notify();
    }

    fn finish_panel_drop(&mut self, at: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        let Some(Gesture::Panel(drag)) = &mut self.gesture else {
            return;
        };
        // The preview and its indices belong to the layout at pickup. Any
        // intervening workspace edit invalidates the drop, including no target.
        if drag.original != self.layout {
            self.cancel_gesture(cx);
            return;
        }
        if let Some(at) = at {
            drag.at = at;
            self.retarget(false);
        }
        let Some(Gesture::Panel(drag)) = self.gesture.take() else {
            return;
        };
        let panels = self.panels(cx);
        let name = self.moving_name(&drag.original, &drag.id, drag.stack, &panels);
        let Some(preview) = drag.preview else {
            let message = match drag.original.drop_outcome(
                &Self::drag_zones(&drag),
                self.dock_bounds,
                (f32::from(drag.at.x), f32::from(drag.at.y)),
                &self.tokens,
                &drag.id,
                drag.stack,
            ) {
                layout::DropOutcome::Here => format!("{name} is already here"),
                _ => format!("Drop on a panel or dock to move {name}"),
            };
            self.notify_status(message, cx);
            return;
        };
        let (near, kind) = self.near_name(preview.near, &drag, &panels);
        let mut next = drag.original;
        if let Err(error) = next.apply_drop(&drag.id, &preview, drag.stack) {
            self.notify_status(error.to_string(), cx);
            return;
        }
        let message = match kind {
            "tab" => format!("{name} tabbed with {near}"),
            "above" => format!("Moved {name} above {near}"),
            "below" => format!("Moved {name} below {near}"),
            _ => format!("Moved {name} → {near}"),
        };
        self.notify_status(message, cx);
        self.commit_panel_drop(next);
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
                        v.end_gesture(None, cx);
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
    fn begin_panel_drag(
        &mut self,
        id: &str,
        at: Point<Pixels>,
        stack: bool,
        w: &Window,
        cx: &mut Context<Self>,
    ) {
        self.guard_panel_drag(cx, |editor, cx| {
            let mut drag = editor.new_drag(id, at, stack, true);
            let label = editor.moving_name(&drag.original, id, stack, &editor.panels(cx));
            drag.chip = Some(still_drag::Chip::new(label, &editor.tokens, w));
            if let Some(host) = &editor.host {
                host.borrow_mut().splitter_focus = false;
            }
            editor.maximized_panel = None;
            editor.gesture = Some(Gesture::Panel(drag));
            editor.split_focus = None;
        });
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
                self.cancel_gesture(cx);
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
                let Some(group) = self.layout.placed(p) else {
                    return false;
                };
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
        if self
            .hidden_panel_headers
            .contains(self.layout.docks[dock].groups[group].active())
        {
            return None;
        }
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
const TAB_SPACING: f32 = pm_design::tab::GAP;
const TAB_EDGE_FADE: f32 = 16.;
const TAB_PADDING: f32 = pm_design::tab::PAD;

/// The tabs any part of which shows at `offset`: the first index and how many.
fn visible_tabs(widths: &[f32], offset: f32, viewport: f32) -> (usize, usize) {
    let (mut first, mut count, mut start) = (None, 0, 0.);
    for (index, width) in widths.iter().enumerate() {
        let end = start + width;
        if end > offset && start < offset + viewport {
            first.get_or_insert(index);
            count += 1;
        }
        start = end + TAB_SPACING;
    }
    (first.unwrap_or(0), count)
}

/// Width of a header title at `size` in the medium weight, as the tab strip draws it.
fn title_width(text: &str, size: f32, window: &Window) -> f32 {
    measure_tab_text(text, size, FontWeight::MEDIUM, window)
}

fn measure_tab_text(text: &str, size: f32, weight: FontWeight, window: &Window) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: Font {
            family: pm_design::TEXT_FONT.into(),
            weight,
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
// The docked trigger has a label, a chevron and two insets. Round up for the
// native text layout.
fn agent_header_width(label: f32, m: &pm_design::Metrics) -> f32 {
    label.ceil() + m.symbol_inline + 4. * m.pill_inset
}

/// Room for a header's tabs: its width less the tools, the ⌄ menu when the
/// tabs overflow, and the restore button of a maximised panel.
fn tab_viewport_width(width: f32, tools: f32, menu: bool, restore: bool, t: &Tokens) -> f32 {
    let m = &t.metrics;
    let gaps = usize::from(menu) + usize::from(tools > 0.) + usize::from(restore);
    let reserved =
        tools + if menu { m.hit } else { 0. } + if restore { m.symbol_inline } else { 0. };
    (width - reserved - t.space.xs * gaps as f32 - t.space.hair).max(0.)
}

/// Measure the string that will actually paint so adjacent labels can keep
/// the same gap even when a tab has room left over after truncation.
fn tab_label(title: &str, natural: f32, room: f32, measure: impl Fn(&str) -> f32) -> (String, f32) {
    if natural <= room {
        return (title.to_owned(), natural);
    }
    let ellipsis = measure("…");
    if ellipsis > room {
        return (String::new(), 0.);
    }
    let fits = |prefix: &str| {
        let label = format!("{}…", prefix.trim_end());
        let width = measure(&label);
        (width <= room).then_some((label, width))
    };
    let mut prefix = title;
    while let Some((words, _)) = prefix.rsplit_once(char::is_whitespace) {
        prefix = words.trim_end();
        if let Some(label) = fits(prefix) {
            return label;
        }
    }
    title
        .grapheme_indices(true)
        .rev()
        .find_map(|(index, _)| fits(&title[..index]))
        .unwrap_or_else(|| ("…".to_owned(), ellipsis))
}

fn panel_switcher_entries(group: &layout::Group, panels: &layout::Panels) -> Vec<MenuEntry> {
    group.tabs.iter().enumerate().map(|(index, id)| {
        MenuEntry::item(panels.title(id), format!("panel:activate:{id}"))
            .checked(index == group.active)
    }).collect()
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
}
impl PanelSlides {
    fn snap(&mut self) {
        self.armed = false;
        self.moving.clear();
        self.clock.rest();
    }
    /// Takes this frame's rects; true while something still travels.
    pub fn place(&mut self, placed: &[(String, Rect)], t: &Tokens) -> bool {
        let now = crate::drag::now();
        let dt = self.clock.tick(now);
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
/// A panel's split/join/dock grip in `corner`, shown while the pointer is in
/// that corner's hit zone: a hairline along the panel's own corner curve (a
/// quad at the panel's corner with its radius and smoothing, bordered on the
/// two sides that meet there), so it is concentric by construction and never
/// leaves the panel's rounded shape.
fn area_grip(
    panel: Rect,
    corner: crate::area::Corner,
    ink: Rgba,
    clip: Option<Rect>,
    t: &Tokens,
) -> impl Fn(&mut Window) + 'static {
    let radius = t.metrics.panel_radius;
    let smoothing = t.radius.smoothing;
    // The corner curve's whole reach, so the stroke is the curve alone.
    let side = (radius * (1. + smoothing)).ceil();
    let x = if corner.left() { panel.x } else { panel.x + panel.w - side };
    let y = if corner.top() { panel.y } else { panel.y + panel.h - side };
    let round = |left: bool, top: bool| {
        if corner.left() == left && corner.top() == top { px(radius) } else { Pixels::ZERO }
    };
    let stroke = |on: bool| if on { px(t.metrics.border) } else { Pixels::ZERO };
    let quad = PaintQuad::new(Bounds::new(point(px(x), px(y)), size(px(side), px(side))))
        .corner_radii(Corners {
            top_left: round(true, true),
            top_right: round(false, true),
            bottom_right: round(false, false),
            bottom_left: round(true, false),
        })
        .corner_smoothing(smoothing)
        .border_widths(Edges {
            top: stroke(corner.top()),
            bottom: stroke(!corner.top()),
            left: stroke(corner.left()),
            right: stroke(!corner.left()),
        })
        .border_color(ink);
    move |window: &mut Window| match clip {
        Some(c) => window.with_content_mask(
            Some(ContentMask {
                bounds: Bounds::new(point(px(c.x), px(c.y)), size(px(c.w), px(c.h))),
                ..Default::default()
            }),
            |window| window.paint_quad(quad.clone()),
        ),
        None => window.paint_quad(quad.clone()),
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
    /// The title bar's empty band, `top` points tall. AppKit passes presses
    /// on the toolbar's empty space through to GPUI's full-size content view,
    /// so a single press starts the window move (`start_window_move`) and a
    /// double-click does what the person chose in System Settings (Fill, Zoom
    /// or Minimize). It paints nothing: the window's ground (and on Home the
    /// sidebar's glass) runs on under the toolbar, so the title bar is the
    /// chrome's one tone, never a third band of its own.
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
                } else if e.click_count == 1 && !e.modifiers.modified() {
                    window.start_window_move();
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
        }
        if let Some(integration) = &self.integration {
            integration.shown_panels(&[], cx);
        }
        let t = &self.tokens;
        let (ground, ground_alpha) = t.ground();
        // The sidebar column's ground is the host's (`Host::sync_sidebar`):
        // the same tint is painted everywhere else, the title bar included.
        let clear = self
            .host
            .as_ref()
            .map_or(0., |h| h.borrow().sidebar_width());
        let host = self.host.clone();
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
                        if let Some(host) = host {
                            host.borrow().stage_native_panels(&NativePanelGeometry::default());
                        }
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
        let scrolling = if matches!(&self.gesture, Some(Gesture::Panel(d)) if d.live && d.moved) {
            self.guard_panel_drag(cx, |editor, _| editor.drag_edge_scroll(w)).unwrap_or(false)
        } else {
            false
        };
        let dragging = self.dragging_panel();
        w.set_pointer_hover_suppressed(dragging);
        let scene = self.scene.clone();
        // Cache the workspace as a sibling of the chip. Pointer and preview
        // frames replace only the overlay; edge scrolling can refresh the scene.
        let scene = if dragging && !scrolling {
            scene.cached(div().size_full().style().clone()).into_any_element()
        } else {
            scene.into_any_element()
        };
        div().relative().size_full().child(scene).children(self.drag_overlay(w, cx))
    }
}

impl Editor {
    fn render_workspace(&mut self, w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
            self.cancel_gesture(cx);
            self.bind_project(id);
        }
        self.sync_workspaces(cx);
        let dragging = self.dragging_panel();
        if !dragging && !matches!(self.gesture, Some(Gesture::Resize(..))) {
            for command in std::mem::take(&mut self.deferred_panel_commands) {
                self.shell_command(&command, cx);
            }
            self.settle_absent_panels(cx);
            for (id, (placement, height)) in &self.agent_build_slots {
                if self.layout.find(id).is_none() {
                    layout::reserve_build(&mut self.layout, id, placement, *height);
                }
            }
        }
        let t = self.tokens.clone();
        let m = t.metrics;
        let c = t.colors.clone();
        let viewport = w.viewport_size();
        let native_top = self
            .host
            .as_ref()
            .map_or(m.toolbar, |h| h.borrow().toolbar_inset());
        let integration = self.integration.clone();
        let extension_menus = integration.as_ref().is_some_and(|i| i.has_extension_context_menus(cx));
        if let Some(host) = &self.host {
            host.borrow().extension_toolbar_context.set(extension_menus);
        }
        let slot = |id: &str, w: &mut Window, cx: &mut Context<Self>| {
            integration
                .as_ref()
                .map_or_else(Vec::new, |i| i.extension_slot(id, &t, w, cx))
        };
        let toolbar_slots =
            ["toolbar.leading", "toolbar.center", "toolbar.trailing"].map(|id| slot(id, w, cx));
        let toolbar_strip = toolbar_slots.iter().any(|zone| !zone.is_empty()).then(|| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(native_top))
                .h(px(m.control))
                .when(extension_menus, |d| d.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, e: &MouseDownEvent, _, cx| {
                        if let Some(i) = this.integration.clone() {
                            i.surface_context_menu("toolbar:context", (e.position.x.into(), e.position.y.into()), cx);
                        }
                    }),
                ))
                .flex()
                .items_center()
                .gap(px(m.gap))
                .px(px(m.inset))
                .overflow_hidden()
                .children(toolbar_slots.into_iter().enumerate().map(
                    |(index, zone)| {
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .items_center()
                            .when(index == 1, |d| d.justify_center())
                            .when(index == 2, |d| d.justify_end())
                            .children(zone)
                    },
                ))
        });
        let top = native_top
            + if toolbar_strip.is_some() {
                m.control
            } else {
                0.
            };
        if let Some(home) = self.integration.clone().and_then(|i| i.home(w, cx)) {
            return self
                .render_home(home, top, cx)
                .children(toolbar_strip)
                .into_any_element();
        }
        let header_mods = header_mods(
            self.integration
                .as_ref()
                .map_or_else(Vec::new, |i| i.panel_header_mods(cx)),
        );
        self.hidden_panel_headers = header_mods
            .iter()
            .filter(|(_, patch)| patch.hidden == Some(true))
            .map(|(id, _)| id.clone())
            .collect();
        // The toolbar band already separates the docks from the window's top
        // edge; the other three sides keep the dock gap.
        self.dock_bounds = Rect {
            x: m.dock_gap,
            y: top,
            w: (f32::from(viewport.width) - m.dock_gap * 2.).max(0.),
            h: (f32::from(viewport.height) - top - m.status - m.dock_gap).max(0.),
        };
        // Freeze content measurements during a splitter drag, so every delta
        // uses the same starting geometry. Explicitly resized heights win below.
        if !dragging && !matches!(self.gesture, Some(Gesture::Resize(..))) {
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
        self.geometry = self.visible_geometry();
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
        if !dragging && focused_handle != self.last_scroll_focus {
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
        if !dragging && self.slides.place(&placed, &t) {
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
        let mut panel_slots = HashMap::new();
        for p in &self.geometry.panels {
            let group = &self.layout.docks[p.dock].groups[p.group];
            let id = group.active();
            if id.eq_ignore_ascii_case("settings")
                || (group.collapsed && id != "viewer" && !self.hidden_panel_headers.contains(id))
            {
                continue;
            }
            let panel_is_native = integration.as_ref().is_some_and(|i| i.native_panel(id, cx));
            let (top_aliases, bottom_aliases) = panel_slot_aliases(id, panel_is_native);
            let slots =
                [("top", top_aliases), ("bottom", bottom_aliases)].map(|(edge, aliases)| {
                    let key = format!("panel:{id}.{edge}");
                    let mut items = slot(&key, w, cx);
                    for alias in aliases {
                        items.extend(slot(alias, w, cx));
                    }
                    let column = slot_column(items, &self.extension_slot_heights, key.clone());
                    let height = self
                        .extension_slot_heights
                        .borrow()
                        .get(&key)
                        .copied()
                        .unwrap_or_default();
                    (column, height)
                });
            panel_slots.insert(id.to_owned(), slots);
        }
        let viewer = self
            .geometry
            .panels
            .iter()
            .find(|p| self.layout.docks[p.dock].groups[p.group].active() == "viewer")
            .map(|p| {
                let header = if self.hidden_panel_headers.contains("viewer") {
                    0.
                } else {
                    m.header
                };
                let [start, end] = panel_slots
                    .get("viewer")
                    .map_or([0., 0.], |slots| [slots[0].1, slots[1].1]);
                Rect {
                    x: p.rect.x + m.border,
                    y: p.rect.y + header + start,
                    w: (p.rect.w - m.border * 2.).max(0.),
                    h: (p.rect.h - header - start - end - m.border).max(0.),
                }
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
            h.publish_menu_state(&self.menu_state_with_focus(w, cx));
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
        // This paint and AppKit share one placement. The background canvas
        // paints first, after the render below has resolved the panel rects.
        let native_geometry = Rc::new(OnceCell::<NativePanelGeometry>::new());
        let paint_geometry = native_geometry.clone();
        let host = self.host.clone();
        #[cfg(test)]
        let native_ground_holes = self.native_ground_holes.clone();
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
                        let panels = paint_geometry.get().expect("panel layout before paint");
                        holes.extend(
                            panels.clips.iter().filter_map(|(_, clip)| {
                                (clip.w > 0. && clip.h > 0.).then_some(*clip)
                            }),
                        );
                        #[cfg(test)]
                        native_ground_holes.replace(holes.clone());
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
                        for (_, body) in &panels.bodies {
                            fill_rounded_off(window, *body, body_radius, [true, true], fill);
                        }
                        // Native/compat panel clips punch holes through `ground`
                        // so the views below show through. Paint the matching
                        // ground gradient over any faded aperture edge so the
                        // native mask blends into the dock ground rather than
                        // exposing raw behind-window vibrancy as a grey band.
                        let fade_depth = pm_design::DOCK_SCROLL_FADE.max(1.);
                        for (id, clip) in &panels.clips {
                            if clip.w <= 0. || clip.h <= 0. {
                                continue;
                            }
                            let Some((_, [top, bottom])) =
                                panels.fades.iter().find(|(owner, _)| owner == id)
                            else {
                                continue;
                            };
                            let top = (*top as f32).clamp(0., clip.h);
                            let bottom = (*bottom as f32).clamp(0., clip.h);
                            if top > 0. {
                                let edge = ground_alpha * (top / fade_depth).clamp(0., 1.);
                                window.paint_quad(
                                    PaintQuad::new(Bounds::new(
                                        point(px(clip.x), px(clip.y)),
                                        gpui::size(px(clip.w), px(top)),
                                    ))
                                    .background(linear_gradient(
                                        180.,
                                        linear_color_stop(tint(ground, edge), 0.),
                                        linear_color_stop(tint(ground, 0.), 1.),
                                    )),
                                );
                            }
                            if bottom > 0. {
                                let edge = ground_alpha * (bottom / fade_depth).clamp(0., 1.);
                                window.paint_quad(
                                    PaintQuad::new(Bounds::new(
                                        point(px(clip.x), px(clip.y + clip.h - bottom)),
                                        gpui::size(px(clip.w), px(bottom)),
                                    ))
                                    .background(linear_gradient(
                                        180.,
                                        linear_color_stop(tint(ground, 0.), 0.),
                                        linear_color_stop(tint(ground, edge), 1.),
                                    )),
                                );
                            }
                        }
                        if let Some(host) = &host {
                            host.borrow().stage_native_panels(panels);
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .on_mouse_move_all({
                let listener = cx.listener(|this, (e, phase): &(MouseMoveEvent, DispatchPhase), _, cx| {
                    if *phase == DispatchPhase::Capture
                        && this.cancelled_press && e.pressed_button.is_some()
                    {
                        cx.stop_propagation();
                        return;
                    }
                    // An already-pressed shell gesture owns motion even over
                    // a panel whose own pointer handlers consume the bubble.
                    if (*phase == DispatchPhase::Capture) != this.gesture.is_some() {
                        return;
                    }
                    let at = (f32::from(e.position.x), f32::from(e.position.y));
                    let hover = (!this.dragging_panel()).then(|| this
                        .geometry
                        .panels
                        .iter()
                        .find(|p| p.rect.contains(at.0, at.1))
                        .and_then(|p| {
                            let id = this.layout.placed(p)?.active().to_string();
                            Some((
                                id,
                                crate::area::Corner::ALL
                                    .into_iter()
                                    .find(|c| c.rect(p.rect).contains(at.0, at.1)),
                            ))
                        })).flatten();
                    if this.area_hover != hover {
                        this.area_hover = hover;
                        cx.notify();
                    }
                    if !e.dragging() {
                        // The button is up: a release that never reached
                        // this window ends the drag here, where it showed.
                        if this.gesture.is_some() {
                            this.end_gesture(None, cx);
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
                            let mut layout = original.clone();
                            if let Some(geometry) = &this.resize_geometry {
                                layout.resize_from_geometry(d, delta, this.dock_bounds, &this.tokens, geometry, true);
                            }
                            if layout != this.layout {
                                this.layout = layout;
                            }
                            cx.notify();
                        }
                        Some(Gesture::Panel(drag)) => {
                            drag.at = e.position;
                            drag.moved |= crate::drag::lifted(
                                (f32::from(drag.start.x), f32::from(drag.start.y)),
                                (f32::from(e.position.x), f32::from(e.position.y)),
                                this.tokens.metrics.panel_drag_threshold,
                            );
                            this.guard_panel_drag(cx, |editor, _| editor.retarget(false));
                            cx.notify();
                        }
                        None => {}
                    }
                });
                move |e, phase, _, w, cx| {
                    listener(&(e.clone(), phase), w, cx);
                }
            })
            .on_mouse_up_all({
                let listener = cx.listener(|this, e: &MouseUpEvent, _, cx| {
                    if e.button == MouseButton::Right && this.cancelled_press {
                        cx.stop_propagation();
                        return;
                    }
                    if e.button != MouseButton::Left {
                        return;
                    }
                    if std::mem::take(&mut this.cancelled_press) {
                        cx.stop_propagation();
                        return;
                    }
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
                    this.end_gesture(Some(e.position), cx);
                });
                move |e: &MouseUpEvent, phase, _, w, cx| {
                    if phase == DispatchPhase::Capture {
                        listener(e, w, cx);
                    }
                }
            })
            .on_mouse_down_all({
                let listener = cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    crate::input::noted();
                    if e.button == MouseButton::Right
                        && (this.gesture.is_some() || this.cancelled_press)
                    {
                        this.cancel_gesture(cx);
                        this.cancelled_press = true;
                        cx.stop_propagation();
                        return;
                    }
                    if e.button == MouseButton::Left {
                        this.cancelled_press = false;
                        // A release that never came ends where the drag showed.
                        this.end_gesture(None, cx);
                    }
                    if this.ringed_panel.take().is_some() {
                        cx.notify();
                    }
                });
                move |e, phase, _, window, cx| {
                    if phase == DispatchPhase::Capture {
                        listener(e, window, cx);
                    }
                }
            })
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
                    this.layout.resize_from_geometry(
                        &d,
                        delta,
                        this.dock_bounds,
                        &this.tokens,
                        &this.geometry,
                        true,
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
        let lifted = match &self.gesture {
            Some(Gesture::Panel(d)) if d.moved && d.live => Some(d),
            _ => None,
        };
        if !dragging {
            self.tab_slides.begin();
        }
        if let Some(integration) = &self.integration {
            integration.open_panels(&self.open_panels(), cx);
        }
        let mut native_panels = NativePanelGeometry::default();
        self.motion.frame(w);
        let kit = crate::kit::Kit::new(&t, &self.motion, |v: &mut Editor| &mut v.motion, w, cx);
        let above_status = Rect {
            x: 0.,
            y: 0.,
            w: f32::from(viewport.width),
            h: (f32::from(viewport.height) - m.status - m.inset).max(0.),
        };
        // Where each header's tabs are drawn, for drops over a header.
        let mut tab_spans: HashMap<(usize, usize), Vec<(usize, f32, f32)>> = HashMap::new();
        #[cfg(test)]
        self.rendered_tab_labels.clear();
        for p in ordered {
            let group = &self.layout.docks[p.dock].groups[p.group];
            let id = group.active().to_string();
            let is_viewer = id == "viewer";
            let native_id = self
                .integration
                .as_ref()
                .filter(|i| i.native_panel(&id, cx))
                .map(|i| i.native_panel_id(&id, cx))
                .filter(|surface| {
                    (surface == &id || !self.panel_order.contains(surface))
                        && !native_panels.bodies.iter().any(|(key, _)| key == surface)
                });
            let tabs = group.tabs.clone();
            let active = group.active.min(tabs.len() - 1);
            // Folded to its header (a double-click on the header).
            let hide_header = self.hidden_panel_headers.contains(&id);
            let header_height = if hide_header { 0. } else { m.header };
            let collapsed = group.collapsed && !is_viewer && !hide_header;
            let multi = tabs.len() > 1;
            let [(top_slot, top_height), (bottom_slot, bottom_height)] =
                panel_slots.remove(&id).unwrap_or([(None, 0.), (None, 0.)]);
            let tab_title = |tab: &str| {
                header_mods
                    .get(tab)
                    .and_then(|patch| patch.title.as_deref())
                    .map(str::to_owned)
                    .unwrap_or_else(|| panels.title(tab).to_owned())
            };
            let tab_decoration = |tab: &str| {
                let patch = header_mods.get(tab);
                let icon = patch.and_then(|p| p.icon.as_deref());
                let badge = patch
                    .and_then(|p| p.badge.as_deref())
                    .filter(|b| !b.is_empty());
                (icon, badge)
            };
            // A stack's tabs are the window's tabs (`pm_design::tab`): a
            // regular title. A lone panel's title is its header's.
            let label_size = t.typography.title;
            let label_weight = if multi {
                FontWeight::NORMAL
            } else {
                FontWeight(t.typography.medium_weight)
            };
            let decoration_widths: Vec<f32> = tabs
                .iter()
                .map(|tab| {
                    let (icon, badge) = tab_decoration(tab);
                    let icon = if icon.is_some() {
                        m.symbol_inline + t.space.xs
                    } else {
                        0.
                    };
                    let badge = badge.map_or(0., |text| {
                        measure_tab_text(text, t.typography.minimum, label_weight, w)
                            + t.space.sm * 2.
                    });
                    icon + badge
                })
                .collect();
            let widths: Vec<f32> = tabs
                .iter()
                .zip(&decoration_widths)
                .map(|(tab, decoration)| {
                    measure_tab_text(&tab_title(tab), label_size, label_weight, w)
                        + decoration
                        + 2. * TAB_PADDING
                })
                .collect();
            let header_rect = self.geometry.header_rect(&p, &t);
            let header_offset = if hide_header {
                0.
            } else {
                header_rect.y - p.rect.y
            };
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
                .elevated(&t, pm_design::Elevation::Panel)
                // During push-off a header can extend into the dock gap.
                // Its body clips separately; the dock still clips both.
                .when(header_offset <= 0., |p| p.overflow_hidden());
            let native_body_visible = native_id.is_some() && !collapsed;
            if let Some(native_id) = native_id.filter(|_| !collapsed) {
                // A native body (an extension's web page) takes the pointer
                // where it shows, so it stops short of the dividers' grab.
                let edge = m.border.max(m.splitter_reach);
                let body = native_body(
                    Rect {
                        h: (shown.h - bottom_height).max(0.),
                        ..shown
                    },
                    edge,
                    header_height + top_height,
                );
                // The slot already includes the dock scroll. Place the body
                // at the animated position, then clip it to the viewport
                // and the travelling sticky header.
                let clip = body.intersection(above_status);
                let clip = self
                    .geometry
                    .side_docks
                    .get(&p.dock)
                    .map_or(clip, |bounds| {
                        let mut clip = clip.intersection(*bounds);
                        let bottom = clip.y + clip.h;
                        let header_bottom = shown.y + header_offset + header_height;
                        clip.y = clip.y.max(header_bottom);
                        clip.h = (bottom - clip.y).max(0.);
                        clip
                    });
                native_panels.clips.push((native_id.clone(), clip));
                native_panels.fades.push((native_id.clone(), self.dock_body_fade(p.dock, clip)));
                native_panels.bodies.push((native_id, body));
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
            // Most custom headers supply a solo title. The Viewer
            // deliberately has none, and the native Scopes header is tools only.
            let own_ground = self
                .integration
                .as_ref()
                .is_some_and(|i| i.paints_own_ground(&id, cx));
            let custom_title = header_mods.get(&id).is_some_and(|patch| {
                patch.title.is_some() || patch.badge.is_some() || patch.icon.is_some()
            });
            let custom = self
                .integration
                .as_ref()
                .filter(|_| !hide_header)
                .and_then(|i| i.header(&id, p.rect, tabs.len() > 1 || custom_title, w, cx));
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
                .rounded_t(px(m.panel_radius))
                .rounded_smoothing(t.radius.smoothing)
                .overflow_hidden()
                // The native viewer body has no panel fill, and the timeline paints
                // its own ground: their headers carry the material.
                .when(
                    (is_viewer && native) || own_ground || header_offset > 0.,
                    |h| h.bg(tint(c.panel, t.translucency.panel)),
                )
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
                        this.begin_panel_drag(&header_id, e.position, true, window, cx);
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
            // Scopes keeps the shell title: its own header overflowed to "S" (PM-668).
            let shell_title = multi || custom.is_none() || id == "scopes" || custom_title;
            if !shell_title {
                header = header.px(px(0.));
            }
            // The strip stands as far in from the panel's edge as its tabs
            // stand from the header's top: the first pill is concentric with
            // the panel's corner.
            let tab_inset = ((m.header - m.control) / 2.).max(0.);
            let tab_radius = t.radius.nested(m.panel_radius, tab_inset);
            if multi {
                header = header
                    .pl(px(tab_inset))
                    .pr(px(t.space.hair))
                    .gap(px(t.space.xs));
            }
            if shell_title {
                // Quiet text tabs (language §9): the active title in `text`, the
                // rest in `muted`; a pill appears only on hover. The whole tab is
                // the drag handle. A single panel shows its title the same way.
                let tools = if custom.is_some() {
                    self.header_tools_widths
                        .borrow()
                        .get(&id)
                        .copied()
                        .unwrap_or_else(|| {
                            if id == "agent" {
                                let label = measure_tab_text(
                                    "Threads",
                                    t.typography.body,
                                    FontWeight::NORMAL,
                                    w,
                                );
                                agent_header_width(label, &m)
                            } else if id == "assets" {
                                // Two view segments, Search, Add, and the header insets.
                                4. * m.hit + 2. * (m.inset + m.pill_inset)
                            } else {
                                2. * m.hit + m.gap
                            }
                        })
                } else if is_viewer {
                    0.
                } else {
                    m.hit
                };
                // The ⌄ menu shows only when the tabs, shrunk evenly to
                // their floors, still do not fit.
                let restore = self.maximized_panel.is_some();
                let gaps = TAB_SPACING * tabs.len().saturating_sub(1) as f32;
                let fits_without =
                    tab_viewport_width(p.rect.w - tab_inset, tools, false, restore, &t);
                let overflow = multi && group.tab_track(fits_without - gaps, &widths).scroll > 0.;
                let viewport = if overflow {
                    tab_viewport_width(p.rect.w - tab_inset, tools, true, restore, &t)
                } else {
                    fits_without
                };
                let track = group.tab_track(viewport - gaps, &widths);
                let scrolling = multi && track.scroll > 0.;
                let content = track.widths.iter().sum::<f32>() + gaps;
                let limit = (content - viewport).max(0.);
                let tab_key = tabs[0].clone();
                let offset = if scrolling {
                    let state = self.tab_scrolls.entry(tab_key.clone()).or_default();
                    state.reveal(&id, active, viewport, &track.widths, limit);
                    state.offset
                } else {
                    self.tab_scrolls.remove(&tab_key);
                    0.
                };
                let clip_width = viewport;
                let (first, visible) = visible_tabs(&track.widths, offset, viewport);
                let single_width = (p.rect.w - tools - m.inset - m.gap * 2.).max(0.);
                let measure_label =
                    |label: &str| measure_tab_text(label, label_size, label_weight, w);
                let labels: Vec<_> = tabs
                    .iter()
                    .enumerate()
                    .map(|(index, tab)| {
                        let room = if multi {
                            track.widths[index] - 2. * TAB_PADDING
                        } else {
                            single_width
                        };
                        tab_label(
                            &tab_title(tab),
                            widths[index] - 2. * TAB_PADDING - decoration_widths[index],
                            room - decoration_widths[index],
                            measure_label,
                        )
                    })
                    .collect();
                #[cfg(test)]
                let window = first..first + visible;
                let mut row = div().flex().items_center().gap(px(TAB_SPACING));
                let mut span_x = p.rect.x + if multi { -offset } else { m.inset };
                let mut spans = vec![];
                // Room the strip has past its tabs: a hovered tab grows into it
                // to show its close button beside its title, never over it.
                let spare = if scrolling {
                    0.
                } else {
                    viewport - track.widths.iter().sum::<f32>() - gaps
                };
                let close_room =
                    pm_design::tab::CLOSE_PAD + pm_design::tab::CLOSE + pm_design::tab::GAP
                        - TAB_PADDING;
                for (ti, tab) in tabs.iter().enumerate() {
                    let closable = layout::closable(tab) && multi;
                    let hover_key = crate::kit::key(&format!("tab-{tab}"));
                    // 0 at rest, 1 under the pointer, on the wash springs.
                    let hovered = if dragging { 0. } else { kit.hovered(hover_key) };
                    let reveal = if closable && !dragging { hovered } else { 0. };
                    let grows = spare >= close_room;
                    let drawn = if multi {
                        track.widths[ti] + if grows { close_room * reveal } else { 0. }
                    } else {
                        single_width
                    };
                    let (label, label_width) = &labels[ti];
                    // Every title starts the tab's padding in: a title never moves.
                    let inset = if multi { TAB_PADDING } else { 0. };
                    let truncated = label.as_str() != tab_title(tab);
                    let tab_left = span_x;
                    let span_width = if multi { drawn } else { *label_width };
                    spans.push((ti, span_x, span_x + span_width));
                    span_x += drawn + TAB_SPACING;
                    // A tab given a new place in its strip slides to it.
                    let slide = self.tab_slides.offset(
                        tab,
                        tab_left + offset + shown.x - p.rect.x,
                        p.rect.y,
                        &t,
                    );
                    #[cfg(test)]
                    if window.contains(&ti) {
                        self.rendered_tab_labels
                            .entry((p.dock, p.group))
                            .or_default()
                            .push(RenderedTabLabel {
                                left: tab_left + inset + slide,
                                right: tab_left + inset + slide + *label_width,
                                font_size: label_size,
                                tab_width: drawn,
                                floor: if ti == active {
                                    widths[ti].min(layout::MAX_ACTIVE_TAB_FLOOR)
                                } else {
                                    layout::MIN_TAB_WIDTH
                                },
                            });
                    }
                    let tabid = tab.clone();
                    let menu_id = tab.clone();
                    let selected = ti == active;
                    let ghost = lifted.is_some_and(|d| d.id == *tab || (d.stack && d.id == id));
                    let close_id = tab.clone();
                    let title = tab_title(tab);
                    // Under the pointer a fainter pill than the showing tab's,
                    // so only one tab ever reads as selected.
                    let fill = if !multi {
                        0.
                    } else if selected {
                        pm_design::tab::FILL
                    } else {
                        pm_design::tab::FILL_HOVER * hovered
                    };
                    let mut cell = div()
                        .id(format!("tab-{tab}"))
                        .role(Role::Tab)
                        .accessibility_id(format!("shell.tab.{tab}"))
                        .aria_selected(selected)
                        .tab_index(0)
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            if event.is_keyboard() {
                                // `p` is from the last layout pass (see `Layout::placed`).
                                if let Some(g) = this
                                    .layout
                                    .docks
                                    .get_mut(p.dock)
                                    .and_then(|d| d.groups.get_mut(p.group))
                                {
                                    g.active = ti;
                                }
                                this.save();
                                cx.notify();
                            }
                        }))
                        .aria_label(title.clone())
                        .relative()
                        .flex_none()
                        .flex()
                        .items_center()
                        .h(px(if multi { m.control } else { m.hit }))
                        .when(multi, |s| {
                            s.rounded(px(tab_radius)).when(fill > 0.004, |s| {
                                s.bg(tint(pm_design::tab::ink(t.scheme), fill))
                            })
                        })
                        .when(slide != 0., |s| s.left(px(slide)))
                        .text_color(if ghost {
                            tint(c.text, 0.4)
                        } else if selected || !multi {
                            tint(c.text, 1.)
                        } else {
                            tint(c.secondary, 1.)
                        })
                        .text_size(px(label_size))
                        .font_family(pm_design::TEXT_FONT)
                        .font_weight(label_weight)
                        .whitespace_nowrap();
                    if multi {
                        cell = kit.track_hover(cell, hover_key);
                    }
                    // A closable tab's close button slides in at its trailing
                    // end with the hover while the title's end fades out under
                    // it (the window's tabs); the title itself never moves.
                    // Grown, the title keeps its whole room. Only in a full
                    // strip does the button stand over the title's end, which
                    // fades out under it (the window's tabs); the title itself
                    // is never cut again.
                    let close_lead = pm_design::tab::CLOSE_PAD + pm_design::tab::CLOSE;
                    let label_right = if grows {
                        TAB_PADDING
                    } else {
                        TAB_PADDING + (close_lead - TAB_PADDING) * reveal
                    };
                    cell = cell
                        .when(multi, |s| s.w(px(drawn)))
                        .when(!multi, |s| s.max_w(px(drawn)))
                        .when(truncated && !dragging, |s| s.tooltip(tooltip(&t, &title)))
                        .child({
                            let (icon, badge) = tab_decoration(tab);
                            let content = div()
                                .flex_none()
                                .when(icon.is_some() || badge.is_some(), |d| {
                                    d.flex().items_center().gap(px(t.space.xs))
                                })
                                .children(icon.map(|icon| {
                                    crate::symbol::symbol(
                                        icon,
                                        m.symbol_inline,
                                        crate::symbol::Weight::Medium,
                                        c.secondary,
                                        w,
                                    )
                                }))
                                .child(label.clone())
                                .children(badge.map(|badge| {
                                    div()
                                        .px(px(t.space.xs))
                                        .rounded(px(m.control_radius))
                                        .bg(rgb(c.selected_fill))
                                        .text_color(rgb(c.accent))
                                        .text_size(px(t.typography.minimum))
                                        .child(badge.to_owned())
                                }));
                            if multi {
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left(px(inset))
                                    .right(px(label_right))
                                    .flex()
                                    .items_center()
                                    .overflow_hidden()
                                    .when(
                                        reveal > 0.004
                                            && *label_width + decoration_widths[ti]
                                                > drawn - inset - label_right,
                                        |d| {
                                            d.overflow_fade(Edges::<AbsoluteLength> {
                                                left: px(0.).into(),
                                                right: px(pm_design::tab::FADE * reveal).into(),
                                                top: px(0.).into(),
                                                bottom: px(0.).into(),
                                            })
                                        },
                                    )
                                    .child(content)
                            } else {
                                content
                            }
                        });
                    // Always there for the keyboard (Tab reaches it, and it
                    // shows while focused); seen only with the hover.
                    if closable && !dragging {
                        let slide = pm_design::tab::CLOSE_SLIDE * (1. - reveal);
                        cell = cell.child(
                            div()
                                .id(format!("close-{tab}"))
                                .aria_label(format!("Close {title}"))
                                .role(Role::Button)
                                .tab_index(0)
                                .absolute()
                                .top(px((m.control - pm_design::tab::CLOSE) / 2.))
                                .right(px(pm_design::tab::CLOSE_PAD - slide))
                                .size(px(pm_design::tab::CLOSE))
                                .rounded(px(t
                                    .radius
                                    .nested(tab_radius, (m.control - pm_design::tab::CLOSE) / 2.)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .opacity(reveal)
                                .focus_style(|s| s.opacity(1.))
                                .hover_style(|s| {
                                    s.bg(tint(pm_design::tab::ink(t.scheme), pm_design::tab::FILL))
                                })
                                .child(crate::symbol::symbol(
                                    "xmark",
                                    (m.symbol_inline - 5.).max(8.),
                                    crate::symbol::Weight::Semibold,
                                    c.secondary,
                                    w,
                                ))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.shell_command(&format!("panel:close:{close_id}"), cx);
                                })),
                        );
                    }
                    if ghost {
                        // The opaque light separator can match the tinted
                        // panel behind it; use tertiary ink over that surface.
                        let outline = if t.scheme == pm_design::Scheme::Light {
                            tint(c.muted, 0.4)
                        } else {
                            tint(c.separator, 1.)
                        };
                        cell = cell.child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded(px(tab_radius))
                                .border(px(1.))
                                .border_dashed()
                                .border_dashed_length(2.5)
                                .border_dashed_gap(1.5)
                                .border_color(outline),
                        );
                    }
                    row = row.child(
                        cell.on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.begin_panel_drag(&tabid, e.position, false, window, cx);
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
                if multi {
                    let strip = div()
                        .id(format!("tab-strip-{}-{}", p.dock, p.group))
                        .relative()
                        .w(px(clip_width))
                        .min_w(px(0.))
                        .flex_shrink(1.)
                        .overflow_hidden()
                        .h(px(m.control))
                        .child(
                            div()
                                .relative()
                                .w(px(clip_width))
                                .h(px(m.control))
                                .overflow_hidden()
                                .overflow_fade(Edges::<AbsoluteLength> {
                                    left: px(TAB_EDGE_FADE * (offset / TAB_EDGE_FADE).clamp(0., 1.))
                                        .into(),
                                    right: px(TAB_EDGE_FADE
                                        * ((limit - offset) / TAB_EDGE_FADE).clamp(0., 1.))
                                    .into(),
                                    top: px(0.).into(),
                                    bottom: px(0.).into(),
                                })
                                .child(row.absolute().top_0().left(px(-offset))),
                        )
                        .on_scroll_wheel(cx.listener(
                            move |this, event: &ScrollWheelEvent, _, cx| {
                                if !scrolling
                                    || this.gesture.is_some()
                                    || event.modifiers.control
                                    || event.modifiers.platform
                                {
                                    return;
                                }
                                let delta = event.delta.pixel_delta(px(16.));
                                // Horizontal trackpad gestures and vertical wheel ticks both
                                // work. A vertical trackpad gesture still scrolls the dock.
                                let by = if delta.x.abs() >= delta.y.abs() {
                                    delta.x
                                } else if matches!(event.delta, gpui::ScrollDelta::Lines(_))
                                    || event.modifiers.shift
                                {
                                    delta.y
                                } else {
                                    return;
                                };
                                if let Some(state) = this.tab_scrolls.get_mut(&tab_key)
                                    && state.scroll(f32::from(by), limit)
                                {
                                    cx.notify();
                                }
                                cx.stop_propagation();
                            },
                        ));
                    header = header.child(strip);
                } else {
                    header = header.child(row);
                }
                if overflow {
                    let entries = panel_switcher_entries(group, &panels);
                    header = header.child(
                        div()
                            .id(format!("switch-panel-{}-{}", p.dock, p.group))
                            .flex_none()
                            .role(Role::Button)
                            .aria_label("Switch Panel")
                            .tab_index(0)
                            .size(px(m.hit))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(m.control_radius))
                            .when(!dragging, |d| d.tooltip(tooltip(&t, "Switch Panel"))
                                .hover_style(|s| s.bg(rgb(c.hover))))
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
            let ext_actions = self
                .integration
                .as_ref()
                .map(|i| i.panel_header_actions(&id, cx))
                .unwrap_or_default();
            let [viewer_left, viewer_right] = if is_viewer && !hide_header {
                ["viewer.controls.left", "viewer.controls.right"].map(|id| slot(id, w, cx))
            } else {
                [vec![], vec![]]
            };
            let has_custom = custom.is_some();
            header = header.children(viewer_left);
            if let Some(custom) = custom {
                header = header.child(
                    div()
                        .when(multi, |d| {
                            let widths = self.header_tools_widths.clone();
                            let id = id.clone();
                            d.flex_none()
                                .ml_auto()
                                .on_children_prepainted(move |children, w, _| {
                                    if let Some(bounds) = children.first() {
                                        let width = f32::from(bounds.size.width).ceil();
                                        if widths.borrow_mut().insert(id.clone(), width) != Some(width) {
                                            w.on_next_frame(|w, _| w.refresh());
                                        }
                                    }
                                })
                        })
                        .when(!multi, |d| d.flex_1().min_w(px(0.)))
                        .h_full()
                        .child(custom),
                );
            }
            header = header.children(viewer_right);
            if !ext_actions.is_empty() {
                let mut strip = div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(t.space.hair))
                    .when(!has_custom, |d| d.ml_auto());
                for action in ext_actions {
                    let cmd = action.command.clone();
                    let tip = action.tooltip.clone();
                    let active = action.active;
                    let has_label = action.label.as_ref().is_some_and(|l| !l.is_empty());
                    let mut btn = div()
                        .id(SharedString::from(format!("ext-hdr-{}-{}", id, action.id)))
                        .role(Role::Button)
                        .aria_label(tip.clone())
                        .h(px(m.hit))
                        .when(!has_label, |d| d.w(px(m.hit)))
                        .when(has_label, |d| d.px(px(t.space.sm)))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap(px(t.space.hair))
                        .rounded(px(m.control_radius))
                        .when(active, |d| {
                            d.bg(rgb(c.selected_fill)).text_color(rgb(c.accent))
                        })
                        .when(!active, |d| d.text_color(rgb(c.secondary)))
                        .when(!dragging, |d| {
                            d.tooltip(tooltip(&t, &tip))
                                .hover_style(|s| s.bg(rgb(c.hover)).text_color(rgb(c.text)))
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, _| {
                            if let Some(host) = &this.host {
                                host.borrow().send_command(&cmd, vec![]);
                            }
                        }));
                    if let Some(icon) = &action.icon {
                        btn = btn.child(crate::symbol::symbol(
                            icon,
                            m.symbol_inline,
                            crate::symbol::Weight::Medium,
                            if active { c.accent } else { c.secondary },
                            w,
                        ));
                    }
                    if let Some(label) = &action.label
                        && !label.is_empty()
                    {
                        btn = btn.child(
                            div()
                                .text_size(px(t.typography.label))
                                .font_family(pm_design::TEXT_FONT)
                                .font_weight(FontWeight::MEDIUM)
                                .whitespace_nowrap()
                                .child(label.clone()),
                        );
                    }
                    strip = strip.child(btn);
                }
                header = header.child(strip);
            }
            if !has_custom && !is_viewer {
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
                        .when(!dragging, |d| d.tooltip(tooltip(&t, &options_title)))
                        .ml_auto()
                        .relative()
                        .size(px(m.hit))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(m.control_radius))
                        .when(!dragging, |d| d.hover_style(|s| s.bg(rgb(c.hover))))
                        .child(crate::symbol::symbol(
                            "ellipsis",
                            m.symbol_panel,
                            crate::symbol::Weight::Medium,
                            c.secondary,
                            w,
                        ))
                        // `text` on hover (language §8): a second glyph fades in.
                        .when(!dragging, |d| d.child(
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
                        ))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                );
            }
            if !dragging && let Some((highlighted, start)) = &self.header_wash {
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
            let stuck_header = if hide_header {
                tab_spans.remove(&(p.dock, p.group));
                None
            } else if header_offset > 0. {
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
                    root = root.child(positioned(*bounds).overflow_hidden().overflow_fade(self.dock_fade_edges(p.dock, header_offset > 0.)).child(panel));
                } else {
                    root = root.child(panel);
                }
                continue;
            }
            if let (Some(i), Some(f)) = (self.integration.clone(), self.panel_focus.get(&id)) {
                let focused = f.contains_focused(w, cx);
                i.panel_focused(&id, focused, cx);
            }
            let body_height = (p.rect.h - header_height - top_height - bottom_height).max(0.);
            // The app's panel callback measures a header-inclusive rect.
            let body_bounds = Rect {
                y: p.rect.y + header_height + top_height - m.header,
                h: body_height + m.header,
                ..p.rect
            };
            let hud = if is_viewer { slot("viewer.hud", w, cx) } else { vec![] };
            let has_slots = top_slot.is_some() || bottom_slot.is_some() || !hud.is_empty();
            let content = self
                .integration
                .as_ref()
                .and_then(|i| i.panel(&id, body_bounds, w, cx));
            let content = if has_slots {
                Some(
                    div()
                        .w_full()
                        .h(px((p.rect.h - header_height).max(0.)))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .children(top_slot)
                        .child(
                            div()
                                .w_full()
                                .relative()
                                .flex_1()
                                .min_h(px(0.))
                                .overflow_hidden()
                                .children(content)
                                .when(!hud.is_empty(), |body| body.child(
                                    div()
                                        .absolute()
                                        .bottom(px(t.space.md))
                                        .left(px(t.space.md))
                                        .right(px(t.space.md))
                                        .flex()
                                        .flex_col()
                                        .gap(px(t.space.xs))
                                        .children(hud),
                                )),
                        )
                        .children(bottom_slot)
                        .into_any_element(),
                )
            } else {
                content
            };
            if let Some(content) = content {
                if let Some(viewport) = self.geometry.side_docks.get(&p.dock) {
                    let height = (p.rect.h - header_height).max(0.);
                    let y = p.rect.y + header_height;
                    let clip_start = header_offset.max(viewport.y - y);
                    let clip_end = height.min(viewport.y + viewport.h - y);
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
            } else if !native_body_visible {
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
                root = root.child(positioned(*bounds).overflow_hidden().overflow_fade(self.dock_fade_edges(p.dock, header_offset > 0.)).child(panel));
            } else {
                root = root.child(panel);
            }
        }
        // Electron v1.2.0 hides side-column scrollbar chrome. Wheel and
        // momentum still move the column; no idle scrollbar timer is needed.
        self.tab_scrolls.retain(|id, _| self.layout.find(id).is_some());
        self.header_tools_widths
            .borrow_mut()
            .retain(|id, _| self.layout.find(id).is_some());
        self.geometry.tabs = tab_spans;
        if !dragging && self.tab_slides.end() {
            w.request_animation_frame();
        }
        if let Some(integration) = &self.integration {
            // Every panel on screen, built-in or an extension's: the viewer
            // and the timeline are not native bodies but are shown.
            let shown: Vec<&str> = self.panel_order.iter().map(String::as_str).collect();
            integration.shown_panels(&shown, cx);
        }
        self.native_body_rects = native_panels.bodies.clone();
        #[cfg(test)]
        {
            self.native_clip_rects = native_panels.clips.clone();
        }
        let _ = native_geometry.set(native_panels);
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
                                || this.layout.reset_split_from_geometry(
                                    &divd,
                                    this.dock_bounds,
                                    &this.tokens,
                                    &this.geometry,
                                    true,
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
                        let next = divd.group.and_then(|g| this.geometry.next_group(divd.dock, g));
                        if this.layout.expand_visible_beside(&divd, next) {
                            this.save();
                        }
                        this.resize_live = false;
                        this.slides.snap();
                        this.tab_slides.snap();
                        this.resize_geometry = Some(this.visible_geometry());
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
                for (i, corner) in crate::area::Corner::ALL.into_iter().enumerate() {
                    let corner_rect = self.geometry.side_docks.get(&p.dock)
                        .map_or(corner.rect(p.rect), |clip| corner.rect(p.rect).intersection(*clip));
                    if corner_rect.w <= 0. || corner_rect.h <= 0. { continue; }
                    let active = self
                        .area_hover
                        .as_ref()
                        .is_some_and(|(h, c)| h == &id && *c == Some(corner));
                    // Only the corner's own hit zone shows its grip; a
                    // plain panel hover shows nothing in the corners.
                    let goal = if active { 1. } else { 0. };
                    let fade = w
                        .use_keyed_transition(
                            format!("area-fade-{id}-{i}"),
                            cx,
                            Duration::from_secs_f64(t.motion.interaction),
                            |_, _| goal,
                        )
                        .with_easing(|p| 1. - (1. - p).powi(3));
                    fade.update(cx, |value, _| *value = goal);
                    let opacity = *fade.evaluate(w, cx);
                    let ink = tint(c.accent, opacity);
                    let id_down = id.clone();
                    let grip = area_grip(
                        p.rect,
                        corner,
                        ink,
                        self.geometry.side_docks.get(&p.dock).copied(),
                        &t,
                    );
                    root = root.child(
                        positioned(corner_rect)
                            .id(format!("area-{id}-{i}"))
                            .tooltip(tooltip(&t, "Drag to split, join or dock"))
                            .cursor(CursorStyle::Crosshair)
                            .child(
                                canvas(move |_, _, _| (), move |_, _, window, _| grip(window))
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
        let (status_items_list, status_mods) = self
            .integration
            .as_ref()
            .map_or_else(Default::default, |i| i.status_bar_extensions(cx));
        let workspace = status_segment(
            &status_mods,
            "workspace",
            workspace::record_name(&self.record).to_owned(),
        );
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
        let left = status_segment(&status_mods, "state", left);
        let offline = status_segment(
            &status_mods,
            "offline",
            self.integration
                .as_ref()
                .map_or_else(String::new, |i| i.status_alert(cx)),
        );
        let dimensions = status_segment(
            &status_mods,
            "dimensions",
            self.integration
                .as_ref()
                .map_or_else(String::new, |i| i.status_trailing(cx)),
        );
        let mut has_center = false;
        let mut status_zones = [
            ("left", "statusBar.left"),
            ("center", "statusBar.center"),
            ("right", "statusBar.right"),
        ]
            .map(|(side, slot_id)| {
                let slots = slot(slot_id, w, cx);
                let items = status_items(&status_items_list, side);
                if slots.is_empty() && items.is_empty() {
                    return None;
                }
                has_center |= side == "center";
                Some(div()
                    .flex()
                    .items_center()
                    .gap(px(m.gap))
                    .min_w(px(0.))
                    .overflow_hidden()
                    .children(items.into_iter().map(|item| {
                        let color = match item.tone.as_str() {
                            "accent" => c.accent,
                            "warning" => c.warning,
                            "danger" => c.danger,
                            _ => c.muted,
                        };
                        let command = item.command.clone();
                        div()
                            .id(SharedString::from(format!("status-{}", item.command)))
                            .flex()
                            .items_center()
                            .gap(px(t.space.xs))
                            .flex_none()
                            .text_color(rgb(color))
                            .rounded(px(m.control_radius))
                            .when(item.clickable, |d| {
                                d.role(Role::Button)
                                    .tab_index(0)
                                    .hover_style(|s| s.bg(rgb(c.hover)))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.command(
                                            Command {
                                                name: command.clone(),
                                                args: vec![],
                                                token: None,
                                            },
                                            window,
                                            cx,
                                        );
                                    }))
                            })
                            .when_some(item.tooltip.as_ref(), |d, tip| d.tooltip(tooltip(&t, tip)))
                            .children(item.icon.as_ref().map(|icon| {
                                crate::symbol::symbol(
                                    icon,
                                    m.symbol_inline,
                                    crate::symbol::Weight::Medium,
                                    color,
                                    w,
                                )
                            }))
                            .child(item.text.clone())
                    }))
                    .children(slots))
            })
            .into_iter();
        root = root
            .child(Self::titlebar_band(native_top))
            .children(toolbar_strip);
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
            .when(extension_menus, |d| d.on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    if let Some(i) = this.integration.clone() {
                        i.surface_context_menu("statusBar:context", (e.position.x.into(), e.position.y.into()), cx);
                    }
                }),
            ))
            .accessibility_id("shell.status")
            .role(Role::Status)
            .aria_label("Status")
            .aria_value(left.clone().unwrap_or_default())
            .flex()
            .items_center()
            .gap(px(m.inset))
            .px(px(m.inset + m.gap))
            .text_size(px(t.typography.minimum))
            .text_color(rgb(c.muted))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(px(m.gap))
                    .children(left.map(|text| div().truncate().child(text)))
                    .children(status_zones.next().flatten()),
            )
            .children(status_zones.next().flatten())
            .child(
                div()
                    .when(has_center, |d| d.flex_1())
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(m.gap))
                    .children(status_zones.next().flatten())
                    .children(offline.filter(|s| !s.is_empty()).map(|alert| {
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
                    }))
                    .children(dimensions.filter(|s| !s.is_empty()).map(|facts| {
                        div()
                            .flex_none()
                            .font_features(FontFeatures(std::sync::Arc::new(vec![(
                                "tnum".into(),
                                1,
                            )])))
                            .child(facts)
                    }))
                    .children(workspace.map(|workspace| {
                        div()
                            .id("status-workspace")
                            .track_focus(&self.workspace_focus)
                            .tab_index(2000)
                            .on_click(cx.listener(|this, e: &ClickEvent, _, _| {
                                this.workspace_menu(e.position())
                            }))
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
                            ))
                    })),
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

    /// The owner closed the Colour panel from its menu and the next pointer
    /// move aborted the app: the move listener read the closed stack through
    /// the last pass's geometry. Every panel closed from its menu, then the
    /// pointer crosses every place the old layout drew, before a redraw.
    #[gpui::test]
    fn closing_any_panel_then_moving_the_pointer_never_panics(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            Editor::new(
                Tokens::new(pm_design::Scheme::Dark, Default::default()),
                None,
                cx,
            )
        });
        cx.run_until_parked();
        let ids: Vec<String> = view.read_with(cx, |v, _| {
            v.layout
                .open_panels()
                .into_iter()
                .filter(|id| crate::layout::closable(id))
                .map(str::to_owned)
                .collect()
        });
        assert!(ids.len() >= 4, "{ids:?}");
        let original = view.read_with(cx, |v, _| v.layout.clone());
        for id in ids {
            let rects: Vec<Rect> =
                view.read_with(cx, |v, _| v.geometry.panels.iter().map(|p| p.rect).collect());
            // As AppKit does after the menu closes: the moves arrive before
            // the window draws the new layout.
            cx.update(|window, cx| {
                view.update(cx, |v, cx| {
                    assert!(v.shell_command(&format!("panel:close:{id}"), cx), "{id}");
                });
                for r in &rects {
                    for (x, y) in [(0.5, 0.5), (0.02, 0.02), (0.98, 0.98), (0.5, 0.05)] {
                        let position = point(px(r.x + r.w * x), px(r.y + r.h * y));
                        window.dispatch_event(
                            PlatformInput::MouseMove(MouseMoveEvent {
                                position,
                                pressed_button: None,
                                modifiers: Modifiers::none(),
                            }),
                            cx,
                        );
                    }
                }
            });
            cx.run_until_parked();
            view.update(cx, |v, cx| {
                v.layout = original.clone();
                cx.notify();
            });
            cx.run_until_parked();
        }
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
    /// The owner: hovering any panel put grey bits in its corners. A plain
    /// panel hover changes no corner pixel; with the pointer in a corner's
    /// hit zone its grip shows, and every pixel it changes lies inside the
    /// panel's rounded shape. Both schemes, every panel, every corner.
    #[::core::prelude::v1::test]
    fn panel_corners_change_only_inside_the_panel_and_only_for_the_grip() {
        for scheme in [pm_design::Scheme::Dark, pm_design::Scheme::Light] {
            let t = Tokens::new(scheme, Default::default());
            // The renderer's continuous corner (normalized superellipse).
            let (radius, smoothing) = (t.metrics.panel_radius, t.radius.smoothing);
            let extent = radius * (1. + smoothing);
            let power = -(2f32.ln())
                / (1. - (1. - std::f32::consts::FRAC_1_SQRT_2) / (1. + smoothing)).ln();
            let outside = |u: f32, v: f32| {
                u < 0.
                    || v < 0.
                    || (u < extent
                        && v < extent
                        && ((extent - u) / extent).powf(power) + ((extent - v) / extent).powf(power)
                            > 1.)
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
                        cx.new(|cx| Editor::new(Tokens::new(scheme, Default::default()), None, cx))
                    })
                    .unwrap();
                cx.run_until_parked();
                let panels: Vec<(String, Rect)> = w
                    .update(&mut cx, |v, _, _| {
                        v.geometry
                            .panels
                            .iter()
                            .map(|p| (v.layout.placed(p).unwrap().active().to_string(), p.rect))
                            .collect()
                    })
                    .unwrap();
                assert!(panels.len() >= 4, "{panels:?}");
                let capture = |cx: &mut HeadlessAppContext, hover| {
                    w.update(cx, |v, _, cx| {
                        v.area_hover = hover;
                        cx.notify();
                    })
                    .unwrap();
                    cx.run_until_parked();
                    // Settle the grip's fade before the frame.
                    std::thread::sleep(Duration::from_millis(150));
                    w.update(cx, |_, _, cx| cx.notify()).unwrap();
                    cx.run_until_parked();
                    cx.capture_screenshot(w.into()).unwrap()
                };
                let rest = capture(&mut cx, None);
                let scale = rest.width() as f32 / 1440.;
                // Pixels within `extent + 4` pt of each corner, as (u, v) in
                // points from the corner (pixel centres) and the largest
                // channel change from rest.
                let changes = |image: &image::RgbaImage, r: Rect, corner: crate::area::Corner| {
                    let n = ((extent + 4.) * scale) as i32;
                    let mut out = vec![];
                    for i in -4..n {
                        for j in -4..n {
                            let (u, v) = ((i as f32 + 0.5) / scale, (j as f32 + 0.5) / scale);
                            let x = if corner.left() { r.x + u } else { r.x + r.w - u };
                            let y = if corner.top() { r.y + v } else { r.y + r.h - v };
                            let (px_, py) = ((x * scale) as u32, (y * scale) as u32);
                            let a = rest.get_pixel(px_, py);
                            let b = image.get_pixel(px_, py);
                            let d = a.0.iter().zip(b.0).map(|(a, b)| a.abs_diff(b)).max().unwrap();
                            out.push((u, v, d));
                        }
                    }
                    out
                };
                for (id, r) in &panels {
                    let hovered = capture(&mut cx, Some((id.clone(), None)));
                    for corner in crate::area::Corner::ALL {
                        let worst = changes(&hovered, *r, corner).into_iter().map(|c| c.2).max();
                        assert!(
                            worst <= Some(2),
                            "{scheme:?} {id} {corner:?}: a plain hover changed the corner ({worst:?})"
                        );
                    }
                    for grip in crate::area::Corner::ALL {
                        let shown = capture(&mut cx, Some((id.clone(), Some(grip))));
                        for corner in crate::area::Corner::ALL {
                            let changed: Vec<_> = changes(&shown, *r, corner)
                                .into_iter()
                                .filter(|c| c.2 > 2)
                                .collect();
                            if corner != grip {
                                assert!(changed.is_empty(), "{scheme:?} {id} {corner:?} changed for {grip:?}");
                                continue;
                            }
                            assert!(changed.len() > 10, "{scheme:?} {id} {grip:?}: no grip");
                            // A changed pixel a whole point outside the shape
                            // is outside it (edge pixels are half covered).
                            for (u, v, d) in changed {
                                assert!(
                                    !outside(u + 1., v + 1.),
                                    "{scheme:?} {id} {grip:?}: ({u:.2}, {v:.2}) outside the panel changed by {d}"
                                );
                            }
                        }
                    }
                }
            });
        }
    }
    #[::core::prelude::v1::test]
    fn area_corners_headless_capture() {
        let Some(dir) = std::env::var_os("PM_PANEL_CORNERS_CAPTURE_DIR") else {
            return;
        };
        // `PM_PANEL_CORNERS_LIGHT=1` captures the light scheme.
        let scheme = if std::env::var_os("PM_PANEL_CORNERS_LIGHT").is_some() {
            pm_design::Scheme::Light
        } else {
            pm_design::Scheme::Dark
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
                            Tokens::new(scheme, Default::default()),
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
                        v.layout.placed(p).unwrap().active().to_string(),
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
                image.save(path.join(format!("{scheme:?}-{name}.png"))).unwrap();
                for (i, c) in crate::area::Corner::ALL.into_iter().enumerate() {
                    // 28 pt from 4 pt outside the corner: the whole curve.
                    let x = if c.left() { r.x - 4. } else { r.x + r.w - 24. };
                    let y = if c.top() { r.y - 4. } else { r.y + r.h - 24. };
                    let (x, y) = ((x.max(0.) * scale) as u32, (y.max(0.) * scale) as u32);
                    image::imageops::crop_imm(
                        &image,
                        x,
                        y,
                        (28. * scale) as u32,
                        (28. * scale) as u32,
                    )
                    .to_image()
                    .save(path.join(format!("{scheme:?}-{name}-corner-{i}.png")))
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

    #[::core::prelude::v1::test]
    fn native_dock_bodies_fade_from_the_visible_aperture_edges() {
        let mut cx = app();
        let window = cx.open_window(size(px(1000.), px(800.)), |_, cx| {
            cx.new(|cx| Editor::new(Tokens::new(pm_design::Scheme::Dark, Default::default()), None, cx))
        }).unwrap();
        window.update(&mut cx, |v, _, _| {
            let dock = v.layout.docks.iter().position(|dock| dock.id == "right").unwrap();
            let viewport = Rect { x: 0., y: 10., w: 100., h: 100. };
            v.geometry.side_docks.insert(dock, viewport);
            v.dock_scroll_max.insert("right".into(), 80.);
            let clip = Rect { y: 15., h: 90., ..viewport };
            // The ramp grows with the offset: a quarter point in fades nothing yet.
            for (offset, expected) in [(0., [0., 23.]), (0.25, [0., 23.]), (40., [23., 23.]), (79.5, [23., 0.]), (80., [23., 0.])] {
                v.dock_scroll.insert("right".into(), offset);
                assert_eq!(v.dock_body_fade(dock, clip), expected);
                let middle = Rect { y: 40., h: 20., ..viewport };
                assert_eq!(v.dock_body_fade(dock, middle), [0., 0.]);
            }
        }).unwrap();
    }

    /// The host removing an extension's panel (disable, uninstall) is not the
    /// person closing it: re-enabling brings back a panel they kept open, and
    /// one they had closed stays closed (w83 P1-4).
    #[::core::prelude::v1::test]
    fn extension_teardown_keeps_the_persons_dismissal_choice() {
        let mut cx = app();
        let window = cx.open_window(size(px(1000.), px(800.)), |_, cx| {
            cx.new(|cx| Editor::new(Tokens::new(pm_design::Scheme::Dark, Default::default()), None, cx))
        }).unwrap();
        window.update(&mut cx, |v, _, cx| {
            let contributions = Rc::new(Contributions(vec![
                ("ext.kept".into(), "Kept".into()),
                ("ext.closed".into(), "Closed".into()),
            ]));
            v.integration = Some(contributions.clone());
            assert!(v.shell_command("panel:restore:ext.kept", cx));
            assert!(v.shell_command("panel:tabwith:ext.kept:ext.closed", cx));
            assert!(v.shell_command("panel:close:ext.closed", cx));
            // Disabled: the host removes both surfaces (the commands
            // extensions.rs sends).
            for id in ["ext.kept", "ext.closed"] {
                v.shell_command(&format!("panel:remove:{id}"), cx);
                assert!(v.layout.find(id).is_none(), "{id} removed");
            }
            let saved = v.record.clone();
            assert_eq!(
                workspace::dismissed_panels(&saved).into_iter().collect::<Vec<_>>(),
                ["ext.closed"],
                "only the person's close is saved"
            );
            // Re-enabled, in this editor and in the next one to restore.
            for reload in [false, true] {
                if reload {
                    v.integration = None;
                    v.show_record(saved.clone());
                    v.integration = Some(contributions.clone());
                }
                for id in ["ext.kept", "ext.closed"] {
                    assert!(v.shell_command(&format!("panel:registered:{id}"), cx));
                    assert!(v.shell_command(&format!("panel:restore:{id}"), cx));
                }
                assert!(v.layout.find("ext.kept").is_some(), "kept panel back, reload {reload}");
                assert!(v.layout.find("ext.closed").is_none(), "closed panel stays closed, reload {reload}");
                v.shell_command("panel:remove:ext.kept", cx);
            }
        }).unwrap();
    }

    #[::core::prelude::v1::test]
    fn dismissed_extension_panels_survive_workspace_reload_until_explicitly_opened() {
        let mut cx = app();
        let window = cx.open_window(size(px(1000.), px(800.)), |_, cx| {
            cx.new(|cx| Editor::new(Tokens::new(pm_design::Scheme::Dark, Default::default()), None, cx))
        }).unwrap();
        window.update(&mut cx, |v, _, cx| {
            let contributions = Rc::new(Contributions(vec![
                ("ext.one".into(), "One".into()),
                ("ext.two".into(), "Two".into()),
            ]));
            v.integration = Some(contributions.clone());
            assert!(v.shell_command("panel:restore:ext.one", cx));
            assert!(v.shell_command("panel:tabwith:ext.one:ext.two", cx));
            assert!(v.shell_command("panel:closeothers:ext.two", cx));
            assert!(v.shell_command("panel:close:ext.two", cx));
            let saved = serde_json::to_vec(&v.record).unwrap();
            v.integration = None;
            v.show_record(serde_json::from_slice(&saved).unwrap());
            assert!(v.shell_command("panel:restore:ext.one", cx));
            v.integration = Some(contributions);
            assert!(v.shell_command("panel:registered:ext.one", cx));
            assert!(v.shell_command("panels:ready", cx));
            for id in ["ext.one", "ext.two"] {
                assert!(v.shell_command(&format!("panel:restore:{id}"), cx));
                assert!(v.layout.find(id).is_none(), "{id} stayed dismissed");
            }
            assert!(v.shell_command("panel:activate:ext.one", cx));
            assert!(v.shell_command("panel:show:ext.two", cx));
            let saved = v.record.clone();
            assert!(workspace::dismissed_panels(&saved).is_empty());
            v.show_record(saved);
            let opened = v.layout.clone();
            assert!(v.shell_command("panel:park:ext.one", cx));
            assert!(v.shell_command("panel:restore:ext.one", cx));
            assert_eq!(v.layout, opened);
            assert!(v.layout.find("ext.two").is_some());
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
                    ["assets", "viewer", "inspector", "fxbrowser", "timeline"]
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
                assert!(v.panel_focus["easing"].is_focused(w));
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
    use super::{Editor, Layout, Rc, Tokens, layout, native_body, workspace};
    use crate::layout::Rect;
    use gpui::{AnyElement, App, Context, IntoElement, ParentElement, Styled, TestAppContext, Window, div, px};
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
        consume_viewer_pointer: std::cell::Cell<bool>,
        native_pexels: std::cell::Cell<bool>,
        panic_on_panel_catalog: std::cell::Cell<bool>,
    }
    struct Stub(Rc<Kernel>);
    impl crate::integration::Integration for Stub {
        fn observe(&self, _: &mut gpui::Context<Editor>) {}
        fn command(&self, _: &crate::native::Command, _: &mut Window, _: &mut App) -> bool {
            false
        }
        fn panel(
            &self,
            id: &str,
            _: crate::layout::Rect,
            _: &mut Window,
            _: &mut App,
        ) -> Option<AnyElement> {
            use gpui::{InteractiveElement, IntoElement, Styled};

            (id == "viewer" && self.0.consume_viewer_pointer.get()).then(|| {
                gpui::div()
                    .size_full()
                    .on_mouse_move(|_, _, cx| cx.stop_propagation())
                    .on_mouse_up(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .into_any_element()
            })
        }
        fn workspaces(&self, _: &App) -> Option<Rc<workspace::Store>> {
            Some(self.0.store.borrow().clone())
        }
        fn write_workspace(&self, write: workspace::Write, _: &mut App) {
            self.0.writes.borrow_mut().push(write);
        }
        fn contributed_panels(&self, _: &App) -> Vec<(String, String)> {
            assert!(!self.0.panic_on_panel_catalog.replace(false), "test panel catalog panic");
            self.0.contributed.borrow().clone()
        }
        fn native_panel(&self, id: &str, _: &App) -> bool {
            id == "fixture.pexels" && self.0.native_pexels.get()
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

    /// PM-613: with no project (Home after the last one closed) Save Workspace
    /// is consumed and saves nothing; Home over an open project, or the
    /// project itself, still saves its layout.
    #[gpui::test]
    fn save_workspace_needs_a_project(cx: &mut TestAppContext) {
        let kernel = Rc::new(Kernel::default());
        kernel.home.set(true);
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let stub = kernel.clone();
        let window = cx.add_window(|_, cx| {
            let mut e = Editor::new(tokens, None, cx);
            e.integration = Some(Rc::new(Stub(stub)));
            e
        });
        window
            .update(cx, |v, _, cx| {
                let before = v.record.clone();
                assert!(v.shell_command("layout:save", cx));
                assert!(v.shell_command("layout:save:No project", cx));
                assert_eq!(v.record, before, "nothing saved without a project");
                kernel.home.set(false);
                assert!(v.shell_command("layout:save:With project", cx));
                assert_eq!(workspace::record_name(&v.record), "With project");
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

    #[gpui::test]
    fn agent_dock_release_after_a_column_loses_a_group(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut v = away_fixture(cx, Rc::new(Kernel::default()));
            v.layout.show_panel("agent", &v.panels(cx)).unwrap();
            v.layout
                .place_relative("agent", "inspector", "group")
                .unwrap();
            v
        });
        cx.run_until_parked();
        let from = view.read_with(cx, |v, _| tab_middle(v, "agent"));
        let outside = (-100., -100.);
        press(cx, from);
        cx.simulate_mouse_move(at(outside), gpui::MouseButton::Left, drag_none());
        let after_close = view.update(cx, |v, cx| {
            assert!(v.dragging_panel());
            assert!(target(v).is_none());
            assert!(v.run_shell_command("panel:close:fxbrowser", cx));
            assert!(v.layout.find("fxbrowser").is_none());
            v.layout.clone()
        });
        cx.run_until_parked();

        cx.simulate_mouse_up(at(outside), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            assert_eq!(v.layout, after_close);
            assert!(v.panel_undo.is_empty());
        });
    }

    #[gpui::test]
    fn agent_dock_floating_release_preserves_a_changed_workspace(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| away_fixture(cx, Rc::new(Kernel::default())));
        cx.run_until_parked();
        let edge = view.read_with(cx, |v, _| {
            let r = stack_rect(v, "assets");
            (r.x + 1., r.y + r.h / 4.)
        });
        let after_close = view.update(cx, |v, cx| {
            assert!(v.drag_from_outside("agent", edge, cx));
            assert!(v.run_shell_command("panel:close:assets", cx));
            assert!(v.layout.find("assets").is_none());
            v.layout.clone()
        });
        cx.run_until_parked();

        view.update(cx, |v, cx| assert!(!v.drop_from_outside("agent", cx)));
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            assert_eq!(v.layout, after_close);
            assert!(v.panel_undo.is_empty());
        });
    }

    #[gpui::test]
    fn agent_dock_floating_project_can_land_on_the_right_without_a_left_dock(
        cx: &mut TestAppContext,
    ) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut v = away_fixture(cx, Rc::new(Kernel::default()));
            v.layout.docks.retain(|d| d.id != "left");
            v
        });
        cx.run_until_parked();
        view.update(cx, |v, cx| {
            let r = stack_rect(v, "inspector");
            let at = (r.x + r.w / 2., r.y + 1.);
            assert!(v.drag_from_outside("assets", at, cx));
            assert!(v.drop_from_outside("assets", cx));
            let (dock, group, _) = v.layout.find("assets").unwrap();
            assert_eq!(v.layout.docks[dock].id, "right");
            assert_eq!(group, 0);
            v.layout.validate().unwrap();
        });
    }

    #[gpui::test]
    fn agent_dock_release_contains_a_panel_catalog_panic(cx: &mut TestAppContext) {
        let kernel = Rc::new(Kernel::default());
        let (view, cx) = cx.add_window_view(|_, cx| away_fixture(cx, kernel.clone()));
        cx.run_until_parked();
        let (before, from, to, edge) = view.read_with(cx, |v, _| {
            let r = stack_rect(v, "assets");
            (
                v.layout.clone(),
                tab_middle(v, "inspector"),
                (r.x + r.w / 2., r.y + r.h / 2.),
                (r.x + 1., r.y + r.h / 4.),
            )
        });
        press(cx, from);
        pull(cx, from, to);
        view.read_with(cx, |v, _| assert!(target(v).is_some()));
        kernel.panic_on_panel_catalog.set(true);
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        assert!(!kernel.panic_on_panel_catalog.get());
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            assert_eq!(v.layout, before);
            assert!(v.panel_undo.is_empty());
        });
        assert!(kernel.writes.borrow().is_empty());

        view.update(cx, |v, cx| {
            assert!(v.drag_from_outside("agent", edge, cx));
            kernel.panic_on_panel_catalog.set(true);
            assert!(!v.drop_from_outside("agent", cx));
        });
        assert!(!kernel.panic_on_panel_catalog.get());
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            assert_eq!(v.layout, before);
            assert!(v.panel_undo.is_empty());
        });
        assert!(kernel.writes.borrow().is_empty());

        view.update(cx, |v, cx| {
            assert!(v.drag_from_outside("agent", edge, cx));
            assert!(v.drop_from_outside("agent", cx));
        });
        view.read_with(cx, |v, _| assert!(v.layout.find("agent").is_some()));
    }

    #[test]
    fn agent_dock_panel_sweep_in_a_short_workspace() {
        use crate::layout::{DropTarget, Edge};
        use gpui::{
            AppContext, HeadlessAppContext, MouseButton, MouseDownEvent, MouseMoveEvent,
            MouseUpEvent, px, size,
        };
        use std::sync::Arc;

        let mut cx = HeadlessAppContext::new(Arc::new(gpui_parley::ParleyTextSystem::new(
            gpui_parley::SystemFonts::Load,
        )));
        for id in [
            "agent",
            "inspector",
            "assets",
            "timeline",
            "fxbrowser",
            "fixture.notes",
        ] {
            let kernel = Rc::new(Kernel::default());
            kernel
                .contributed
                .borrow_mut()
                .push(("fixture.notes".into(), "Fixture Notes".into()));
            let window = cx
                .open_window(size(px(1000.), px(330.)), |_, cx| {
                    cx.new(|cx| {
                        let mut v = away_fixture(cx, kernel.clone());
                        let record = serde_json::json!({"id": "short-docks", "layout": {"docks": [
                            {"id": "left", "size": 240, "panels": [{"id": "notes", "flex": true}]},
                            {"id": "center", "panels": [{"id": "viewer", "flex": true}]},
                            {"id": "right", "size": 330, "panels": [
                                {"id": id, "flex": true}, {"id": "shader", "size": 500}
                            ]}
                        ]}});
                        *kernel.store.borrow_mut() = Rc::new(workspace::Store {
                            project: Some(record.clone()),
                            project_id: Some("p".into()),
                            ..Default::default()
                        });
                        v.seen = Some(record.clone());
                        v.show_record(record);
                        v
                    })
                })
                .unwrap();
            cx.run_until_parked();
            let original = window.update(&mut cx, |v, _, _| v.layout.clone()).unwrap();

            for zone in ["top", "centre", "bottom", "edge"] {
                window
                    .update(&mut cx, |v, _, cx| {
                        v.layout = original.clone();
                        cx.notify();
                    })
                    .unwrap();
                cx.run_until_parked();
                let (from, to, left) = window
                    .update(&mut cx, |v, _, _| {
                        let r = stack_rect(v, "notes");
                        let body_top = r.y + v.tokens.metrics.header;
                        let fraction = match zone {
                            "top" => 0.1,
                            "centre" => 0.5,
                            _ => 0.85,
                        };
                        let viewer = stack_rect(v, "viewer");
                        let to = if zone == "edge" {
                            (
                                viewer.x + viewer.w / 2.,
                                v.dock_bounds.y + v.dock_bounds.h + 1.,
                            )
                        } else {
                            (r.x + r.w / 2., body_top + (r.y + r.h - body_top) * fraction)
                        };
                        (tab_middle(v, id), to, v.layout.find("notes").unwrap().0)
                    })
                    .unwrap();
                cx.simulate_event(
                    window.into(),
                    MouseDownEvent {
                        position: at(from),
                        button: MouseButton::Left,
                        click_count: 1,
                        ..Default::default()
                    },
                );
                cx.simulate_event(
                    window.into(),
                    MouseMoveEvent {
                        position: at(to),
                        pressed_button: Some(MouseButton::Left),
                        modifiers: drag_none(),
                    },
                );
                let expected = window.update(&mut cx, |v, _, _| {
                    let preview = target(v).expect("the drop has a preview");
                    match zone {
                        "top" => assert_eq!(preview, DropTarget::Split { dock: left, group: 0 }),
                        "centre" => assert!(matches!(preview, DropTarget::Tab { dock, group: 0, .. } if dock == left)),
                        "bottom" => assert_eq!(preview, DropTarget::Split { dock: left, group: 1 }),
                        _ => assert_eq!(preview, DropTarget::Edge(Edge::Bottom)),
                    }
                    assert_eq!(v.layout, original, "{id} {zone}: still dragging");
                    preview
                }).unwrap();
                cx.simulate_event(
                    window.into(),
                    MouseUpEvent {
                        position: at(to),
                        button: MouseButton::Left,
                        ..Default::default()
                    },
                );
                window
                    .update(&mut cx, |v, _, _| {
                        assert!(v.gesture.is_none(), "{id} {zone}: drag ended");
                        assert!(v.slides.moving.is_empty() && !v.tab_slides.travelling());
                        v.layout.validate().unwrap();
                        let (dock, group, _) = v.layout.find(id).unwrap();
                        match expected {
                            DropTarget::Tab { .. } => {
                                assert_eq!(Some(group), v.layout.find("notes").map(|p| p.1))
                            }
                            DropTarget::Split {
                                group: expected, ..
                            } => assert_eq!(group, expected),
                            _ => {}
                        }
                        assert_eq!(
                            v.layout.docks[dock].id,
                            if zone == "edge" { "bottom" } else { "left" }
                        );
                    })
                    .unwrap();
            }

            window
                .update(&mut cx, |v, _, cx| {
                    v.layout = original.clone();
                    v.layout.close_panel(id).unwrap();
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            window
                .update(&mut cx, |v, _, cx| {
                    let before = v.layout.clone();
                    let viewer = stack_rect(v, "viewer");
                    let centre = (viewer.x + viewer.w / 2., viewer.y + viewer.h / 2.);
                    assert!(
                        !v.drag_from_outside(id, centre, cx),
                        "{id}: centre keeps it floating"
                    );
                    assert!(!v.drop_from_outside(id, cx));
                    assert_eq!(v.layout, before);
                    let left = stack_rect(v, "notes");
                    let edge = (left.x + 1., left.y + left.h / 4.);
                    assert!(
                        v.drag_from_outside(id, edge, cx),
                        "{id}: left edge previews"
                    );
                    assert!(v.drop_from_outside(id, cx));
                    let (dock, _, _) = v.layout.find(id).unwrap();
                    assert_eq!(v.layout.docks[dock].id, "left");
                    v.layout.validate().unwrap();
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn agent_dock_release_panic_preserves_the_workspace_record(cx: &mut TestAppContext) {
        let kernel = Rc::new(Kernel::default());
        let (view, cx) = cx.add_window_view(|_, cx| away_fixture(cx, kernel.clone()));
        let (before, record, from, to) = view.read_with(cx, |v, _| {
            let r = stack_rect(v, "assets");
            (
                v.layout.clone(),
                v.workspace_record().clone(),
                tab_middle(v, "inspector"),
                (r.x + r.w / 2., r.y + r.h / 2.),
            )
        });
        press(cx, from);
        pull(cx, from, to);
        let hold = view.update(cx, |v, _| {
            assert!(target(v).is_some());
            // Fail after the move has been planned, while creating its status notice.
            std::mem::replace(&mut v.tokens.motion.note_hold, f64::NAN)
        });
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        view.update(cx, |v, _| {
            v.tokens.motion.note_hold = hold;
            assert!(v.gesture.is_none());
            assert_eq!(v.layout, before);
            assert_eq!(v.workspace_record(), &record);
            assert!(v.outbox.is_empty() && v.pending.is_empty());
            assert!(v.panel_undo.is_empty());
        });
        assert!(kernel.writes.borrow().is_empty());
    }

    #[gpui::test]
    fn agent_dock_releasing_a_stack_keeps_its_tabs_and_active_panel(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut v = away_fixture(cx, Rc::new(Kernel::default()));
            v.layout.show_panel("agent", &v.panels(cx)).unwrap();
            v.layout
                .place_relative("agent", "inspector", "group")
                .unwrap();
            v
        });
        cx.run_until_parked();
        let (from, to) = view.read_with(cx, |v, _| {
            let header = v.panel_header_rect("agent").unwrap();
            let last = v.tab_rect("agent").unwrap();
            let viewer = stack_rect(v, "viewer");
            (
                (last.x + last.w + 8., header.y + header.h / 2.),
                (viewer.x + viewer.w / 2., viewer.y + viewer.h * 0.8),
            )
        });
        press(cx, from);
        pull(cx, from, to);
        view.read_with(cx, |v, _| {
            assert!(matches!(&v.gesture, Some(super::Gesture::Panel(d)) if d.stack && d.moved));
            assert!(target(v).is_some());
        });
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            let (d, g, _) = v.layout.find("agent").unwrap();
            assert_eq!(v.layout.docks[d].id, "center");
            let stack = &v.layout.docks[d].groups[g];
            assert_eq!(stack.tabs, ["inspector", "agent"]);
            assert_eq!(stack.active(), "agent");
            assert!(v.gesture.is_none());
        });
    }

    #[gpui::test]
    fn panel_calm_drag_leaves_the_source_layout_intact_until_release(cx: &mut TestAppContext) {
        let tokens = Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility {
            reduce_motion: true,
            ..Default::default()
        });
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        view.update(cx, |v, _| {
            let before = v.layout.clone();
            let viewer = v.panel_rect("viewer").unwrap();
            v.simulate_drag("inspector", viewer.x + viewer.w * 0.5, viewer.y + viewer.h * 0.85);
            assert!(matches!(&v.gesture, Some(super::Gesture::Panel(d)) if d.preview.is_some()));
            assert_eq!(v.layout, before);
            assert!(v.simulate_drop().is_some());
            assert_ne!(v.layout, before);
        });
    }

    fn away_fixture(cx: &mut gpui::Context<Editor>, kernel: Rc<Kernel>) -> Editor {
        let mut v = Editor::new(
            Tokens::new(
                pm_design::Scheme::Dark,
                pm_design::Accessibility { reduce_motion: true, ..Default::default() },
            ),
            None,
            cx,
        );
        let record = serde_json::json!({"id": "away-fixture", "layout": {"docks": [
            {"id": "left", "size": 260, "panels": [{"id": "assets", "flex": true}]},
            {"id": "center", "panels": [{"id": "viewer", "flex": true}]},
            {"id": "right", "size": 300, "panels": [
                {"id": "inspector", "flex": true}, {"id": "fxbrowser", "size": 320}
            ]}
        ]}});
        *kernel.store.borrow_mut() = Rc::new(workspace::Store {
            project: Some(record.clone()),
            project_id: Some("p".into()),
            ..Default::default()
        });
        v.integration = Some(Rc::new(Stub(kernel)));
        v.project = "p".into();
        v.seen = Some(record.clone());
        v.show_record(record);
        v
    }

    #[gpui::test]
    fn panel_away_stacked_divider_owns_motion_across_viewer(cx: &mut TestAppContext) {
        let kernel = Rc::new(Kernel::default());
        kernel.consume_viewer_pointer.set(true);
        let (view, cx) = cx.add_window_view(|_, cx| away_fixture(cx, kernel));
        cx.run_until_parked();
        let from = view.read_with(cx, |v, _| {
            let d = v.layout.find("inspector").unwrap().0;
            let r = v.geometry.dividers.iter().find(|s| s.dock == d && !s.horizontal).unwrap().rect;
            (r.x + r.w / 2., r.y + 100.)
        });
        let to = (from.0 - 75., from.1);
        press(cx, from);
        cx.simulate_mouse_move(at(to), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            let (d, _, _) = v.layout.find("inspector").unwrap();
            assert_eq!(v.layout.docks[d].groups.len(), 2);
            assert_eq!(v.layout.docks[d].size, Some(375.), "viewer must not eat divider motion");
        });
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none(), "release must finish resize");
            let (d, _, _) = v.layout.find("inspector").unwrap();
            assert_eq!(v.layout.docks[d].size, Some(375.));
        });
    }

    #[gpui::test]
    fn panel_away_release_commits_even_where_viewer_consumes_mouse_up(cx: &mut TestAppContext) {
        let kernel = Rc::new(Kernel::default());
        kernel.consume_viewer_pointer.set(true);
        let (view, cx) = cx.add_window_view(|_, cx| away_fixture(cx, kernel));
        cx.run_until_parked();
        let (before, to) = view.read_with(cx, |v, _| {
            let r = stack_rect(v, "viewer");
            (v.layout.clone(), (r.x + r.w / 2., r.y + r.h * 0.8))
        });
        view.update(cx, |v, cx| {
            v.simulate_drag("fxbrowser", to.0, to.1);
            assert!(target(v).is_some(), "fixture has a valid target");
            cx.notify();
        });
        cx.run_until_parked();
        let released = std::time::Instant::now();
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            assert_ne!(v.layout, before, "drop commits in the release dispatch");
        });
        eprintln!("PANEL_AWAY release-to-observed-commit {:?}", released.elapsed());
    }

    #[gpui::test]
    fn panel_away_shell_undo_restores_the_layout_before_a_drop(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| away_fixture(cx, Rc::new(Kernel::default())));
        cx.run_until_parked();
        let (before, start, to) = view.read_with(cx, |v, _| {
            let r = stack_rect(v, "viewer");
            (v.layout.clone(), tab_middle(v, "fxbrowser"), (r.x + r.w / 2., r.y + r.h * 0.8))
        });
        press(cx, start);
        pull(cx, start, to);
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        let dropped = view.read_with(cx, |v, _| {
            assert_ne!(v.layout, before, "fixture drop must land");
            v.layout.clone()
        });
        cx.update(|w, cx| {
            view.update(cx, |v, cx| {
                v.command(crate::native::Command {
                    name: "undo".into(), args: vec![], token: None,
                }, w, cx);
                assert_eq!(v.layout, before);
                v.command(crate::native::Command {
                    name: "contextRedo".into(), args: vec![], token: None,
                }, w, cx);
                assert_eq!(v.layout, dropped);
                w.focus(&v.panel_focus["viewer"], cx);
                v.command(crate::native::Command {
                    name: "contextUndo".into(), args: vec![], token: None,
                }, w, cx);
                assert_eq!(v.layout, dropped, "panel content is not workspace focus");
            });
        });
    }

    #[gpui::test]
    fn panel_away_show_extension_from_menu_reveals_its_body(cx: &mut TestAppContext) {
        let kernel = Rc::new(Kernel::default());
        kernel.contributed.borrow_mut().push(("fixture.pexels".into(), "Pexels".into()));
        let (view, cx) = cx.add_window_view(|_, cx| away_fixture(cx, kernel));
        cx.run_until_parked();
        view.update(cx, |v, cx| {
            let (d, g, _) = v.layout.find("inspector").unwrap();
            v.layout.docks[d].groups[g].size = Some(1800.);
            assert!(v.shell_command("panel:toggle:fixture.pexels", cx));
        });
        cx.run_until_parked();
        view.read_with(cx, |v, cx| {
            assert!(v.menu_state(cx).open.iter().any(|id| id == "fixture.pexels"));
            let dock = v.layout.find("fixture.pexels").unwrap().0;
            let viewport = v.geometry.side_docks[&dock];
            let group = stack_rect(v, "fixture.pexels");
            assert!(group.intersection(viewport).h > v.tokens.metrics.header,
                "a newly checked panel must be brought into the viewport");
        });
    }

    #[gpui::test]
    fn panel_away_revealed_web_panel_has_one_native_body_at_each_resize_width(cx: &mut TestAppContext) {
        let kernel = Rc::new(Kernel::default());
        kernel.native_pexels.set(true);
        kernel.contributed.borrow_mut().push(("fixture.pexels".into(), "Pexels".into()));
        let (view, cx) = cx.add_window_view(|_, cx| away_fixture(cx, kernel));
        view.update(cx, |v, cx| {
            let (d, g, _) = v.layout.find("inspector").unwrap();
            v.layout.docks[d].groups[g].size = Some(1800.);
            assert!(v.shell_command("panel:toggle:fixture.pexels", cx));
        });
        cx.run_until_parked();
        let body = |v: &Editor| {
            let slots: Vec<_> = v.native_body_rects.iter()
                .filter(|(id, _)| id == "fixture.pexels").map(|(_, r)| *r).collect();
            assert_eq!(slots.len(), 1, "the shown extension owns exactly one native body");
            slots[0]
        };
        let (start, initial) = view.read_with(cx, |v, _| {
            let dock = v.layout.find("fixture.pexels").unwrap().0;
            let r = v.geometry.dividers.iter().find(|s| s.dock == dock && !s.horizontal).unwrap().rect;
            let initial = body(v);
            ((r.x + r.w / 2., initial.y + 10.), initial)
        });
        press(cx, start);
        for delta in [32., 57., 72.] {
            let to = (start.0 - delta, start.1);
            cx.simulate_mouse_move(at(to), gpui::MouseButton::Left, drag_none());
            cx.run_until_parked();
            view.read_with(cx, |v, _| assert_eq!(body(v).w, initial.w + delta));
        }
        cx.simulate_mouse_up(at((start.0 - 72., start.1)), gpui::MouseButton::Left, drag_none());
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert_eq!(body(v).w, initial.w + 72.));
    }

    #[gpui::test]
    fn panel_away_unavailable_solo_group_does_not_take_dock_space(cx: &mut TestAppContext) {
        const RETIRED: &str = "fixture.retired-import";
        let kernel = Rc::new(Kernel::default());
        let stub = kernel.clone();
        let (view, cx) = cx.add_window_view(|_, cx| away_fixture(cx, stub));
        view.update(cx, |v, cx| {
            let mut record = v.record.clone();
            record["layout"]["docks"][0]["panels"].as_array_mut().unwrap().push(
                serde_json::json!({"id": RETIRED,"size":356}),
            );
            v.show_record(record);
            cx.notify();
        });
        cx.run_until_parked();
        view.read_with(cx, |v, _| assert!(v.panel_rect(RETIRED).is_some(), "wait for discovery"));
        view.update(cx, |v, cx| { assert!(v.shell_command("panels:ready", cx)); });
        cx.run_until_parked();
        view.read_with(cx, |v, cx| {
            assert!(workspace::layout(&v.record).find(RETIRED).is_some(),
                "keep the saved slot for an owner that may return");
            assert!(v.panel_rect(RETIRED).is_none());
            assert!(!v.menu_state(cx).open.iter().any(|id| id == RETIRED));
        });
        kernel.contributed.borrow_mut().push((RETIRED.into(), "Import".into()));
        view.update(cx, |v, cx| {
            assert!(v.shell_command(&format!("panel:registered:{RETIRED}"), cx));
        });
        cx.run_until_parked();
        view.read_with(cx, |v, cx| {
            assert!(v.menu_state(cx).open.iter().any(|id| id == RETIRED));
            assert_eq!(v.layout.find(RETIRED), Some((0, 1, 0)));
            assert_eq!(v.panel_rect(RETIRED).unwrap().h, 356.);
        });
    }

    #[gpui::test]
    fn dock_tabs_keep_all_tabs_through_divider_motion(cx: &mut TestAppContext) {
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        view.update(cx, |v, cx| {
            v.layout = Layout::preset("design");
            let (d, g, _) = v.layout.find("inspector").unwrap();
            v.layout.docks[d].size = Some(215.);
            let group = &mut v.layout.docks[d].groups[g];
            group.tabs.extend(["anchor", "three-long-extra-tabs", "four-extra-tabs"].map(str::to_owned));
            cx.notify();
        });
        cx.run_until_parked();
        let (from, key) = view.read_with(cx, |v, _| {
            let (dock, group, _) = v.layout.find("inspector").unwrap();
            let r = v.geometry.dividers.iter().find(|d| d.dock == dock && !d.horizontal).unwrap().rect;
            ((r.x + r.w / 2., r.y + 100.), (dock, group))
        });
        let visible = |v: &Editor| v.geometry.tabs[&key].iter().map(|s| s.0).collect::<Vec<_>>();
        let initial = view.read_with(cx, |v, _| visible(v));
        assert_eq!(initial, [0, 1, 2, 3], "all tabs must be laid out at rest");
        press(cx, from);
        view.read_with(cx, |v, _| assert_eq!(visible(v), initial));
        let wider = (from.0 - 170., from.1);
        pull(cx, from, wider);
        view.read_with(cx, |v, _| assert_eq!(visible(v), initial));
        pull(cx, wider, (from.0 + 135., from.1));
        view.read_with(cx, |v, _| {
            assert!(v.resize_live);
            assert_eq!(visible(v), initial);
        });
    }

    #[::core::prelude::v1::test]
    fn dock_tabs_selection_reveals_the_active_tab() {
        let mut scroll = super::TabScroll::default();
        let widths = [72., 72., 72., 72., 64.];
        let content: f32 = widths.iter().sum::<f32>() + 4. * super::TAB_SPACING;
        let max = content - 250.;
        scroll.reveal("pexels", 4, 250., &widths, max);
        assert_eq!(scroll.offset, max, "the last tab ends at the viewport's right edge");
        // Scrolling follows the trackpad point for point, never by whole tabs.
        assert!(scroll.scroll(3.5, max));
        assert_eq!(scroll.offset, max - 3.5);
        scroll.reveal("pexels", 4, 250., &widths, max);
        assert_eq!(scroll.offset, max - 3.5, "manual scrolling is kept for the same selection");
        scroll.reveal("properties", 0, 250., &widths, max);
        assert_eq!(scroll.offset, 0.);
        assert!(!scroll.scroll(10., max), "the offset stops at the first tab");
        assert!(scroll.scroll(-1000., max));
        assert_eq!(scroll.offset, max, "and at the last");
    }

    #[::core::prelude::v1::test]
    fn dock_tabs_visible_range_follows_a_fractional_offset() {
        let widths = [72., 72., 120., 72., 64.];
        assert_eq!(super::visible_tabs(&widths, 0., 150.), (0, 2));
        let (first, count) = super::visible_tabs(&widths, 40.5, 150.);
        assert_eq!(first, 0, "a tab partly scrolled out still shows");
        assert!(count >= 2);
        let end: f32 = widths.iter().sum::<f32>() + 4. * super::TAB_SPACING;
        assert_eq!(super::visible_tabs(&widths, end - 64., 64.), (4, 1));
    }

    #[::core::prelude::v1::test]
    fn dock_tabs_switcher_lists_every_panel_in_order_and_marks_the_selection() {
        let group = crate::layout::Group {
            tabs: ["inspector", "fxbrowser", "notes", "captions", "easing"]
                .map(str::to_owned).into(),
            active: 2,
            size: None,
            collapsed: false,
        };
        let items = super::panel_switcher_entries(&group, &crate::layout::Panels::default());
        assert_eq!(items.len(), group.tabs.len());
        for (index, (item, id)) in items.iter().zip(&group.tabs).enumerate() {
            assert_eq!(item.command, format!("panel:activate:{id}"));
            assert_eq!(item.checked, index == group.active);
        }
    }

    #[gpui::test]
    fn panel_calm_release_saves_width_through_an_async_workspace_publication(cx: &mut TestAppContext) {
        let mut record = workspace::preset("design").unwrap();
        record["id"] = "resize".into();
        record["builtin"] = false.into();
        let kernel = Rc::new(Kernel::default());
        *kernel.store.borrow_mut() = Rc::new(workspace::Store {
            records: workspace::seeded(std::slice::from_ref(&record)),
            project: Some(record.clone()),
            project_id: Some("p".into()),
            ..Default::default()
        });
        *kernel.project.borrow_mut() = Some("p".into());
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(Tokens::new(pm_design::Scheme::Dark, Default::default()), None, cx);
            e.project = "p".into();
            e.integration = Some(Rc::new(Stub(kernel.clone())));
            e
        });
        cx.run_until_parked();
        let from = view.read_with(cx, |v, _| {
            let (dock, _, _) = v.layout.find("inspector").unwrap();
            let r = v.geometry.dividers.iter().find(|d| d.dock == dock && !d.horizontal).unwrap().rect;
            (r.x + r.w / 2., r.y + 100.)
        });
        let width = |l: &Layout| l.docks.iter().find(|d| d.id == "right").unwrap().size.unwrap();
        let initial = view.read_with(cx, |v, _| width(&v.layout));
        press(cx, from);
        let to = (from.0 - 72., from.1);
        pull(cx, from, to);
        let live = view.read_with(cx, |v, _| width(&v.layout));
        assert!(live > initial);
        record["name"] = "Renamed elsewhere".into();
        let store = (**kernel.store.borrow()).clone();
        *kernel.store.borrow_mut() = Rc::new(workspace::Store {
            project: Some(record),
            ..store
        });
        view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        view.read_with(cx, |v, _| {
            assert_eq!(width(&v.layout), live);
            let persisted = serde_json::to_vec(v.workspace_record()).unwrap();
            let relaunched = workspace::layout(&serde_json::from_slice(&persisted).unwrap());
            assert!((width(&relaunched) - live).abs() < 0.5);
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
    #[gpui::test]
    fn the_agent_launcher_follows_the_active_workspace_tab(cx: &mut TestAppContext) {
        let frames = Rc::new(RefCell::new(vec![]));
        let record = frames.clone();
        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let window = cx.add_window(|_, cx| {
            let mut editor = Editor::new(tokens, None, cx);
            editor.integration = Some(Rc::new(Shown(record)));
            editor
        });
        let launcher_shown = || {
            let frames = frames.borrow();
            let shown = frames.last().expect("a frame");
            crate::agent_launcher::show_agent_button(true, shown.iter().any(|id| id == "agent"))
        };

        window
            .update(cx, |editor, _, cx| {
                assert!(editor.run_shell_command("panel:tabwith:inspector:agent", cx));
                assert!(editor.run_shell_command("panel:activate:inspector", cx));
                let (dock, group, _) = editor.layout.find("agent").expect("Agent is docked");
                let (inspector_dock, inspector_group, _) =
                    editor.layout.find("inspector").expect("Properties is docked");
                assert_eq!((dock, group), (inspector_dock, inspector_group));
            })
            .unwrap();
        cx.run_until_parked();
        assert!(launcher_shown(), "Agent is behind Properties");

        window
            .update(cx, |editor, _, cx| {
                assert!(editor.run_shell_command("panel:activate:agent", cx));
            })
            .unwrap();
        cx.run_until_parked();
        assert!(!launcher_shown(), "Agent is the active tab");
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
    /// Advance the deterministic frame clock with the pointer still held.
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

    #[gpui::test]
    fn right_click_cancels_a_dock_tab_drag_and_a_splitter_resize(cx: &mut TestAppContext) {
        use gpui::MouseButton;

        let tokens = Tokens::new(pm_design::Scheme::Dark, Default::default());
        let (view, cx) = cx.add_window_view(|_, cx| Editor::new(tokens, None, cx));
        cx.run_until_parked();
        let before = view.read_with(cx, |v, _| v.layout.clone());
        let writes = view.read_with(cx, |v, _| v.outbox.len());
        let from = view.read_with(cx, |v, _| tab_middle(v, "inspector"));
        let target = view.read_with(cx, |v, _| stack_rect(v, "assets"));
        let to = (target.x + target.w / 2., target.y + target.h * 0.9);
        let cancel = |cx: &mut gpui::VisualTestContext, at| {
            cx.simulate_mouse_down(at, MouseButton::Right, drag_none());
            cx.simulate_mouse_up(at, MouseButton::Right, drag_none());
            cx.simulate_mouse_move(at, MouseButton::Left, drag_none());
            cx.simulate_mouse_up(at, MouseButton::Left, drag_none());
        };
        press(cx, from);
        pull(cx, from, to);
        assert!(view.read_with(cx, |v, _| v.dragging_panel()));
        cancel(cx, at(to));
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            assert_eq!(v.layout, before);
            assert_eq!(v.outbox.len(), writes);
        });

        let d = view.read_with(cx, |v, _| {
            v.geometry.dividers.iter().find(|d| !d.horizontal).unwrap().clone()
        });
        let from = (d.rect.x + d.rect.w / 2., d.rect.y + d.rect.h / 2.);
        let to = (from.0 + 50., from.1);
        press(cx, from);
        pull(cx, from, to);
        view.read_with(cx, |v, _| {
            assert!(v.resize_live);
            assert!(v.layout != before);
        });
        cancel(cx, at(to));
        view.read_with(cx, |v, _| {
            assert!(v.gesture.is_none());
            assert!(!v.resize_live);
            assert_eq!(v.layout, before);
            assert_eq!(v.outbox.len(), writes);
        });
    }

    /// Each kind of drop (a tab position, a column position beside a panel,
    /// a dock's edge): only the overlay previews the drop. The release
    /// writes once. A cancel, or a release back home, writes nothing.
    #[gpui::test]
    fn a_drag_previews_the_destination_and_lands_as_one_change(cx: &mut TestAppContext) {
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
            assert_eq!(view.read_with(cx, |v, _| v.layout.clone()), before, "{kind}: in passing");
            rest(cx, &view);
            let (shown, aimed) = view.read_with(cx, |v, _| {
                assert_eq!(v.layout, before, "{kind}: even after a pause");
                let Some(super::Gesture::Panel(d)) = &v.gesture else { panic!("no drag") };
                let preview = d.preview.unwrap();
                let mut shown = before.clone();
                shown.apply_drop(id, &preview, false).unwrap();
                (shown, Some(preview.target))
            });
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
            assert_eq!(v.layout, before);
            v.cancel_gesture(cx);
            assert_eq!(v.layout, before, "a cancel restores the layout");
            assert_eq!(v.outbox.len(), written, "and writes nothing");
            assert!(!v.dragging_panel(), "the chip and preview disappear instantly");
            assert!(!v.slides.armed, "cancel leaves the layout still");
        });
        cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
        cx.run_until_parked();
        // (The panel is back in its place before it is pressed again.)
        arrive(cx, &view);
        // Dragged out and brought back home: released there, nothing changes.
        press(cx, from);
        pull(cx, from, to);
        rest(cx, &view);
        assert_eq!(view.read_with(cx, |v, _| v.layout.clone()), before);
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
    /// with hysteresis: a pointer just over a boundary keeps the target it had.
    #[gpui::test]
    fn a_drag_target_holds_at_a_boundary(cx: &mut TestAppContext) {
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
        // Just over the line and back, nothing changes.
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
        // A menu move can still animate. Dragging it takes ownership
        // from the command at the header's actual painted location.
        view.update(cx, |v, cx| assert!(v.run_shell_command("panel:below:inspector:fxbrowser", cx)));
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
    struct NativeBody {
        aliases: bool,
    }
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
            id == "inspector" || (self.aliases && id == "assets")
        }
        fn native_panel_id(&self, _: &str, _: &App) -> String {
            "inspector".into()
        }
        fn project_id(&self, _: &App) -> Option<String> {
            None
        }
        fn status(&self, _: &App) -> String {
            String::new()
        }
        fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    }

    #[gpui::test]
    fn panel_calm_native_slot_is_published_once_per_body(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(Tokens::new(pm_design::Scheme::Dark, Default::default()), None, cx);
            e.integration = Some(Rc::new(NativeBody { aliases: true }));
            e
        });
        cx.run_until_parked();
        view.read_with(cx, |v, _| {
            assert_eq!(v.native_body_rects.iter().filter(|(id, _)| id == "inspector").count(), 1);
        });
    }

    /// The native body and its shell header occupy the same slot on pickup,
    /// through target changes and on the first frame of a committed drop.
    #[gpui::test]
    fn dock_tabs_drag_drop_presents_final_body_in_one_frame(
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
                e.integration = Some(Rc::new(NativeBody { aliases: false }));
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
            rest(cx, &view);
            view.read_with(cx, |v, _| {
                assert_eq!(stack_rect(v, "inspector"), home);
                assert_eq!(body(v), super::native_body(home, edge, m.header));
                assert!(target(v).is_some(), "one destination is available");
            });
            cx.simulate_mouse_up(at(to), gpui::MouseButton::Left, drag_none());
            cx.run_until_parked();
            view.read_with(cx, |v, _| {
                let place = stack_rect(v, "inspector");
                assert_ne!(place, home, "only the release moved it");
                let shown = v.slides.shown("inspector", place);
                assert_eq!(shown, place, "the first drop frame is at the destination");
                assert_eq!(body(v), super::native_body(shown, edge, m.header));
                assert!(v.slides.moving.is_empty() && !v.tab_slides.travelling());
                assert!(!v.dragging_panel());
            });
            crate::drag::set_frame_time(None);
        }
    }

    /// The aperture a native body shows through stays inside its side dock's
    /// viewport: an extension's page can never reach the window's status bar,
    /// even when the column's groups stand taller than it and the panel's own
    /// bottom is below the fold.
    #[gpui::test]
    fn native_body_clip_stays_above_the_status_bar(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| {
            let mut e = Editor::new(
                Tokens::new(pm_design::Scheme::Dark, Default::default()),
                None,
                cx,
            );
            e.integration = Some(Rc::new(NativeBody { aliases: false }));
            e
        });
        // The column outgrows its viewport: a saved group height taller than
        // the dock makes it scroll, like the list that painted under the
        // status bar.
        view.update(cx, |v, cx| {
            let right = v
                .layout
                .docks
                .iter_mut()
                .find(|dock| dock.id == "right")
                .unwrap();
            right.groups = vec![layout::Group {
                tabs: vec!["inspector".to_owned()],
                active: 0,
                size: Some(2400.),
                collapsed: false,
            }];
            cx.notify();
        });
        cx.run_until_parked();
        for scroll in [0., 220., 900.] {
            view.update(cx, |v, cx| {
                v.dock_scroll.insert("right".into(), scroll);
                cx.notify();
            });
            cx.run_until_parked();
            view.read_with(cx, |v, _| {
                let m = v.tokens.metrics;
                let status_top = v.dock_bounds.y + v.dock_bounds.h + m.dock_gap;
                let (dock, _, _) = v.layout.find("inspector").unwrap();
                let viewport = v.geometry.side_docks[&dock];
                let panel = v
                    .geometry
                    .panels
                    .iter()
                    .find(|p| p.dock == dock)
                    .expect("the panel is placed");
                assert!(
                    panel.rect.y + panel.rect.h <= viewport.y + viewport.h + 0.01,
                    "scroll {scroll}: panel bottom {} is below the column viewport {}",
                    panel.rect.y + panel.rect.h,
                    viewport.y + viewport.h,
                );
                let (_, body) = v
                    .native_body_rects
                    .iter()
                    .find(|(id, _)| id == "inspector")
                    .expect("the native body is placed");
                assert!(
                    body.y + body.h <= status_top - m.inset + 0.01,
                    "scroll {scroll}: content bottom {} reached the status bar inset {}",
                    body.y + body.h,
                    status_top - m.inset,
                );
                let (_, clip) = v
                    .native_clip_rects
                    .iter()
                    .find(|(id, _)| id == "inspector")
                    .expect("the native body publishes a clip");
                assert!(
                    clip.y + clip.h <= viewport.y + viewport.h + 0.01,
                    "scroll {scroll}: clip bottom {} passed the column viewport {}",
                    clip.y + clip.h,
                    viewport.y + viewport.h,
                );
            });
        }
    }

    /// Paint the editor in the same headless compositor used for dock
    /// captures and check the aperture on every frame of a column scroll.
    struct ScrollingDockFixture;
    impl crate::integration::Integration for ScrollingDockFixture {
        fn observe(&self, _: &mut Context<Editor>) {}
        fn command(&self, _: &crate::native::Command, _: &mut Window, _: &mut App) -> bool {
            false
        }
        fn panel(&self, id: &str, rect: Rect, _: &mut Window, _: &mut App) -> Option<AnyElement> {
            let label = match id {
                "inspector" => "Property",
                "easing" => "Easing preset",
                "fxbrowser" => "Effect",
                "fixture.anchor-point" => "Anchor layer",
                _ => return None,
            };
            Some(
                div()
                    .h(px(rect.h))
                    .flex()
                    .flex_col()
                    .children((1..=14).map(|row| {
                        div()
                            .h(px(24.))
                            .flex_none()
                            .px(px(12.))
                            .child(format!("{label} · row {row}"))
                    }))
                    .into_any_element(),
            )
        }
        fn native_panel(&self, id: &str, _: &App) -> bool {
            matches!(id, "easing" | "fixture.anchor-point")
        }
        fn native_panel_id(&self, id: &str, _: &App) -> String {
            id.into()
        }
        fn contributed_panels(&self, _: &App) -> Vec<(String, String)> {
            vec![("fixture.anchor-point".into(), "Anchor Point".into())]
        }
        fn project_id(&self, _: &App) -> Option<String> {
            None
        }
        fn status(&self, _: &App) -> String {
            "Playback paused".into()
        }
        fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    }

    #[::core::prelude::v1::test]
    fn scrolled_extension_bodies_stay_inside_their_panels() {
        use gpui::AppContext;
        use std::{path::PathBuf, sync::Arc};

        let mut failures = Vec::new();
        for (theme, scheme) in [
            ("dark", pm_design::Scheme::Dark),
            ("light", pm_design::Scheme::Light),
        ] {
            let mut cx = gpui::HeadlessAppContext::with_platform(
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
            let window = cx
                .open_window(gpui::size(px(1000.), px(660.)), |_, cx| {
                    cx.new(|cx| {
                        let mut editor =
                            Editor::new(Tokens::new(scheme, Default::default()), None, cx);
                        editor.integration = Some(Rc::new(ScrollingDockFixture));
                        let right = editor
                            .layout
                            .docks
                            .iter_mut()
                            .find(|d| d.id == "right")
                            .unwrap();
                        right.groups = [
                            "inspector",
                            "easing",
                            "fxbrowser",
                            "fixture.anchor-point",
                        ]
                        .into_iter()
                        .map(|id| layout::Group {
                            tabs: vec![id.into()],
                            active: 0,
                            size: Some(420.),
                            collapsed: false,
                        })
                        .collect();
                        editor
                    })
                })
                .unwrap();
            cx.run_until_parked();
            let maximum = window
                .update(&mut cx, |v, _, _| v.dock_scroll_max["right"])
                .unwrap();
            assert!(maximum > 100., "the stack must scroll: max {maximum}");
            let mut saw_visible = [false; 2];
            let mut saw_hidden = [false; 2];
            for step in 0..=20 {
                let scroll = maximum * step as f32 / 20.;
                window
                    .update(&mut cx, |v, _, cx| {
                        v.dock_scroll.insert("right".into(), scroll);
                        cx.notify();
                    })
                    .unwrap();
                cx.run_until_parked();
                if matches!(step, 0 | 5 | 10 | 15 | 20)
                    && let Some(dir) = std::env::var_os("PM_DOCK_RENDER_CAPTURE")
                {
                    let image = cx.capture_screenshot(window.into()).unwrap();
                    let dir = PathBuf::from(dir);
                    std::fs::create_dir_all(&dir).unwrap();
                    image
                        .save(dir.join(format!("{theme}-extension-scroll-{step:02}.png")))
                        .unwrap();
                }
                window.update(&mut cx, |v, _, _| {
                    let m = v.tokens.metrics;
                    let status_inset = v.dock_bounds.y + v.dock_bounds.h + m.dock_gap - m.inset;
                    let dock = v.layout.find("inspector").unwrap().0;
                    let viewport = v.geometry.side_docks[&dock];
                    let holes = v.native_ground_holes.borrow();
                    let mut visible = Vec::new();
                    for (index, id) in ["easing", "fixture.anchor-point"].into_iter().enumerate() {
                        let (dock, group, _) = v.layout.find(id).unwrap();
                        let panel = v.geometry.panels.iter()
                            .find(|p| (p.dock, p.group) == (dock, group))
                            .unwrap_or_else(|| panic!("{id} placed"))
                            .rect;
                        let (_, clip) = *v.native_clip_rects.iter()
                            .find(|(key, _)| key == id)
                            .unwrap_or_else(|| panic!("{id} clip published"));
                        let content = native_body(panel, m.border.max(m.splitter_reach), m.header)
                            .intersection(viewport);
                        let offscreen = content.w <= 0. || content.h <= 0.;
                        saw_hidden[index] |= offscreen;
                        if clip.w > 0. && clip.h > 0. {
                            saw_visible[index] = true;
                            visible.push(clip);
                            if offscreen
                                || clip.x < content.x - 0.01
                                || clip.y < content.y - 0.01
                                || clip.x + clip.w > content.x + content.w + 0.01
                                || clip.y + clip.h > content.y + content.h + 0.01
                                || clip.y + clip.h > status_inset + 0.01
                            {
                                failures.push(format!(
                                    "{theme}/{step} {id}: clip {clip:?} outside panel content {content:?}"
                                ));
                            }
                        }
                    }
                    if *holes != visible {
                        failures.push(format!(
                            "{theme}/{step}: GPUI opened {holes:?} for WebKit's {visible:?}"
                        ));
                    }
                }).unwrap();
            }
            assert_eq!(saw_visible, [true; 2]);
            assert_eq!(saw_hidden, [true; 2]);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
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
    fn dock_tabs_review_captures() {
        let Some(dir) = std::env::var_os("PM_DOCK_TABS_CAPTURE") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (scheme, name) in [(pm_design::Scheme::Light, "light"), (pm_design::Scheme::Dark, "dark")] {
            let mut cx = HeadlessAppContext::with_platform(
                Arc::new(gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)),
                Arc::new(()),
                || Some(Box::new(gpui_apple::metal_renderer::MetalHeadlessRenderer::new())),
            );
            let tokens = Tokens::new(scheme, pm_design::Accessibility {
                reduce_motion: true,
                ..Default::default()
            });
            let window = cx.open_window(size(px(1280.), px(720.)), |_, cx| cx.new(|cx| {
                let mut editor = Editor::new(tokens, None, cx);
                editor.layout = Layout::preset("design");
                let right = editor.layout.docks.iter_mut().find(|dock| dock.id == "right").unwrap();
                right.groups[0] = layout::Group {
                    tabs: ["inspector", "anchor-point", "remove-background", "commons-logos", "pexels"].map(str::to_owned).into(),
                    active: 4,
                    size: Some(200.),
                    collapsed: false,
                };
                editor
            })).unwrap();
            for (index, width) in [550., 420., 330., 260., 180., 260.].into_iter().enumerate() {
                window.update(&mut cx, |editor, _, cx| {
                    let dock = editor.layout.docks.iter_mut().find(|dock| dock.id == "right").unwrap();
                    dock.size = Some(width);
                    dock.groups[0].size = Some(if index == 5 { 88. } else { 200. });
                    cx.notify();
                }).unwrap();
                cx.run_until_parked();
                window
                    .update(&mut cx, |editor, _, _| {
                        let (dock, group, _) = editor.layout.find("pexels").unwrap();
                        let labels = &editor.rendered_tab_labels[&(dock, group)];
                        assert!(
                            labels
                                .iter()
                                .all(|label| label.font_size == editor.tokens.typography.title)
                        );
                        if index < 2 {
                            assert_eq!(labels.len(), 5, "all five labels fit at {width} pt");
                        }
                        if labels.len() < 5 {
                            assert!(
                                labels
                                    .iter()
                                    .all(|label| label.tab_width <= label.floor + 0.01),
                                "compressed width at {width} pt"
                            );
                        }
                        let gaps: Vec<_> = labels
                            .windows(2)
                            .map(|pair| pair[1].left - pair[0].right)
                            .collect();
                        let spread = if gaps.is_empty() {
                            0.
                        } else {
                            let tightest = gaps.iter().copied().fold(f32::INFINITY, f32::min);
                            let widest = gaps.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                            widest - tightest
                        };
                        assert!(spread <= 1., "label gaps at {width} pt: {gaps:?}");
                        println!("{name} {width:.0} pt: {} visible tabs, {:.0} pt labels, {spread:.2} pt gap spread",
                            labels.len(), editor.tokens.typography.title);
                    })
                    .unwrap();
                if index == 4 {
                    let menu = window.update(&mut cx, |editor, _, cx| {
                        let (dock, group, _) = editor.layout.find("pexels").unwrap();
                        let group = &editor.layout.docks[dock].groups[group];
                        panel_switcher_entries(group, &editor.panels(cx))
                    }).unwrap();
                    assert_eq!(menu.len(), 5);
                    assert_eq!(menu.iter().filter(|item| item.checked).count(), 1);
                    assert_eq!(menu.last().unwrap().command, "panel:activate:pexels");
                    let rows: Vec<_> = menu.iter().map(|item| (&item.label, item.checked)).collect();
                    std::fs::write(dir.join(format!("{name}-switcher.json")),
                        serde_json::to_vec(&rows).unwrap()).unwrap();
                }
                let rect = window.update(&mut cx, |editor, _, _| editor.panel_rect("pexels").unwrap()).unwrap();
                let screenshot = cx.capture_screenshot(window.into()).unwrap();
                let scale = screenshot.width() as f32 / 1280.;
                let crop_height = rect.h + if index == 5 { 50. } else { 0. };
                let crop = image::imageops::crop_imm(&screenshot,
                    (rect.x * scale).round() as u32,
                    (rect.y * scale).round() as u32,
                    (rect.w * scale).round() as u32,
                    (crop_height * scale).round() as u32,
                ).to_image();
                let file = if index == 5 { format!("{name}-short-stack.png") } else { format!("{name}-width-{index}.png") };
                crop.save(dir.join(file)).unwrap();
            }
        }
    }

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionStatusBarItem {
    pub id: String,
    pub side: String,
    pub order: i32,
    pub text: String,
    pub icon: Option<String>,
    pub tooltip: Option<String>,
    pub tone: String,
    pub clickable: bool,
    pub command: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtensionStatusBarMod {
    pub target: String,
    pub hidden: Option<bool>,
    pub text: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelHeaderMod {
    pub panel_id: String,
    pub title: Option<String>,
    pub icon: Option<String>,
    pub badge: Option<String>,
    pub hidden: Option<bool>,
}

fn status_segment(mods: &[ExtensionStatusBarMod], target: &str, default: String) -> Option<String> {
    let mut hidden = false;
    let mut text = None;
    for patch in mods.iter().filter(|patch| patch.target == target) {
        hidden = patch.hidden.unwrap_or(hidden);
        text = patch.text.as_ref().or(text);
    }
    (!hidden).then(|| text.cloned().unwrap_or(default))
}

fn status_items<'a>(
    items: &'a [ExtensionStatusBarItem],
    side: &str,
) -> Vec<&'a ExtensionStatusBarItem> {
    let mut items: Vec<_> = items.iter().filter(|item| item.side == side).collect();
    items.sort_by_key(|item| item.order);
    items
}

fn header_mods(patches: Vec<PanelHeaderMod>) -> HashMap<String, PanelHeaderMod> {
    let mut mods: HashMap<String, PanelHeaderMod> = HashMap::new();
    for patch in patches {
        let id = &patch.panel_id;
        if id.eq_ignore_ascii_case("settings") {
            continue;
        }
        if let Some(old) = mods.get_mut(id) {
            old.title = patch.title.or(old.title.take());
            old.icon = patch.icon.or(old.icon.take());
            old.badge = patch.badge.or(old.badge.take());
            old.hidden = patch.hidden.or(old.hidden);
        } else {
            mods.insert(id.clone(), patch);
        }
    }
    mods
}

fn panel_slot_aliases(
    id: &str,
    native: bool,
) -> (&'static [&'static str], &'static [&'static str]) {
    // GPUI bodies mount the original slots themselves. A native replacement
    // needs the shell to host the equivalent slots outside its native view.
    match id {
        "inspector" if native => (
            &["inspector.top", "inspector.header"],
            &["inspector.bottom"],
        ),
        "assets" if native => (&["assets.top", "assets.toolbar"], &["assets.bottom"]),
        "agent" if native => (&["agent.top", "agent.toolbar"], &["agent.bottom"]),
        "timeline" if native => (
            &["timeline.top", "timeline.header", "timeline.toolbar"],
            &["timeline.bottom", "timeline.footer"],
        ),
        "inspector" => (&["inspector.header"], &[]),
        "assets" => (&["assets.top"], &["assets.bottom"]),
        "fxbrowser" => (&["effects.top"], &["effects.bottom"]),
        "layer-effects" => (&["layerEffects.top"], &["layerEffects.bottom"]),
        "agent" => (&["agent.top"], &["agent.bottom"]),
        "easing" => (&["easing.top"], &["easing.bottom"]),
        "color" => (&["color.top"], &["color.bottom"]),
        "timeline" => (&["timeline.top", "timeline.header"], &["timeline.bottom"]),
        "viewer" => (&["viewer.top"], &["viewer.bottom"]),
        _ => (&[], &[]),
    }
}

fn slot_column(
    items: Vec<AnyElement>,
    heights: &Rc<RefCell<HashMap<String, f32>>>,
    id: String,
) -> Option<Div> {
    if items.is_empty() {
        heights.borrow_mut().remove(&id);
        return None;
    }
    let heights = heights.clone();
    Some(
        div()
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .child(div().w_full().flex().flex_col().children(items))
            .on_children_prepainted(move |bounds, w, _| {
                let height = bounds
                    .first()
                    .map_or(0., |bounds| f32::from(bounds.size.height));
                let mut heights = heights.borrow_mut();
                if (heights.get(&id).copied().unwrap_or_default() - height).abs() > 0.5 {
                    heights.insert(id.clone(), height);
                    w.on_next_frame(|w, _| w.refresh());
                }
            }),
    )
}

#[cfg(test)]
mod extension_chrome_tests {
    use super::*;

    #[test]
    fn status_items_keep_registration_order_for_ties_and_clear() {
        let item = |id: &str, side: &str, order| ExtensionStatusBarItem {
            id: id.into(),
            side: side.into(),
            order,
            text: id.into(),
            icon: None,
            tooltip: None,
            tone: "default".into(),
            clickable: false,
            command: String::new(),
        };
        let items = vec![
            item("first", "left", 0),
            item("second", "left", 0),
            item("center", "center", -3),
            item("before", "left", -1),
        ];
        assert_eq!(
            status_items(&items, "left")
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["before", "first", "second"]
        );
        assert_eq!(status_items(&items, "center")[0].id, "center");
        assert!(status_items(&[], "left").is_empty());

        let mods = [
            ExtensionStatusBarMod {
                target: "offline".into(),
                hidden: Some(true),
                text: None,
            },
            ExtensionStatusBarMod {
                target: "state".into(),
                hidden: None,
                text: Some("Ready".into()),
            },
        ];
        assert_eq!(status_segment(&mods, "offline", "Offline".into()), None);
        assert_eq!(
            status_segment(&mods, "state", "Saved".into()).as_deref(),
            Some("Ready")
        );
        assert_eq!(
            status_segment(&[], "offline", "Offline".into()).as_deref(),
            Some("Offline")
        );
        assert_eq!(
            status_segment(&[], "state", "Saved".into()).as_deref(),
            Some("Saved")
        );
    }

    #[test]
    fn panel_header_mods_merge_and_clear_with_settings_protected() {
        let mods = header_mods(vec![
            PanelHeaderMod {
                panel_id: "inspector".into(),
                title: Some("Properties".into()),
                icon: Some("slider.horizontal.3".into()),
                ..Default::default()
            },
            PanelHeaderMod {
                panel_id: "inspector".into(),
                badge: Some("New".into()),
                hidden: Some(true),
                ..Default::default()
            },
            PanelHeaderMod {
                panel_id: "Settings".into(),
                hidden: Some(true),
                ..Default::default()
            },
        ]);
        let inspector = &mods["inspector"];
        assert_eq!(inspector.title.as_deref(), Some("Properties"));
        assert_eq!(inspector.icon.as_deref(), Some("slider.horizontal.3"));
        assert_eq!(inspector.badge.as_deref(), Some("New"));
        assert_eq!(inspector.hidden, Some(true));
        assert_eq!(mods.len(), 1);
        assert!(header_mods(vec![]).is_empty());
    }
}
