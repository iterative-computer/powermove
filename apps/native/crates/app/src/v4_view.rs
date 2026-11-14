//! GPUI host for accepted pm-surface trees. Publications compile on a worker;
//! render reads compiled facts only and never waits on JS, the kernel or disk.
//! Ordinary controls deliver events to JS; native surfaces own their own input.
use crate::v4_compile::{Kit, NodeView, Prepared, TokenTable};
use gpui::{prelude::*, *};
use pm_builtins::{Builtins, Publication};
use pm_shell::text::{TextEvent, TextField, TextKind};
use pm_surface::{
    Id,
    events::{DragData, DragEffect, EventDetail},
    kit::Component,
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

/// Native surfaces by the opaque `surface` lease a `canvas`/`frame` node mounts.
#[path = "v4_agent.rs"]
pub mod agent;

#[derive(Clone, Default)]
pub struct Natives {
    pub views: Rc<RefCell<BTreeMap<String, AnyView>>>,
    pub outline: Rc<Outline>,
    /// The window's latest publication: what host-listed pop-up rows
    /// (`HostRows`) are read from when a menu opens.
    pub published: Rc<RefCell<Option<Arc<pm_kernel::Published>>>>,
}
/// Scroll and column geometry the timeline surface shares with its JSX outline.
/// The outline row window stays JS; its position follows native scroll at once.
#[derive(Default)]
pub struct Outline {
    scroll_y: Cell<f32>,
    pub column: Cell<f32>,
    scene: RefCell<Option<Arc<pm_panel_timeline::render::Scene>>>,
    view: RefCell<Option<WeakEntity<SurfaceView>>>,
    surface: RefCell<Option<WeakEntity<crate::timeline_surface::TimelineSurface>>>,
}
impl Outline {
    pub fn bind_surface(&self, surface: WeakEntity<crate::timeline_surface::TimelineSurface>) {
        *self.surface.borrow_mut() = Some(surface);
    }
    /// A timeline value readout drags as one kernel gesture on the surface.
    fn scrub_property(&self, layer: &str, path: &str, at: Point<Pixels>, cx: &mut App) -> bool {
        self.surface
            .borrow()
            .as_ref()
            .and_then(|s| s.upgrade())
            .is_some_and(|s| {
                s.update(cx, |s, _| s.press_property(layer.into(), path.into(), at));
                true
            })
    }
    pub fn set_scene(&self, scene: Option<Arc<pm_panel_timeline::render::Scene>>) {
        *self.scene.borrow_mut() = scene;
    }
    pub fn bind(&self, view: WeakEntity<SurfaceView>) {
        *self.view.borrow_mut() = Some(view);
    }
    fn offset(&self, first: &str) -> f32 {
        let top = self
            .scene
            .borrow()
            .as_ref()
            .and_then(|s| s.scroll_for(first))
            .unwrap_or(0.) as f32;
        top - self.scroll_y.get()
    }
    pub fn scrolled(&self, scroll_y: f32, column: f32, rows: Option<(usize, usize)>, cx: &mut App) {
        let moved = self.scroll_y.get() != scroll_y || self.column.get() != column;
        self.scroll_y.set(scroll_y);
        self.column.set(column);
        if let Some(view) = self.view.borrow().as_ref().and_then(|v| v.upgrade()) {
            view.update(cx, |v, cx| {
                if moved {
                    cx.notify();
                }
                if let Some(rows) = rows {
                    v.outline_rows(rows, cx);
                }
            });
        }
    }
}
/// The only pointer the host models. Multi-touch reaches panels as gestures.
const POINTER: u64 = 1;

#[path = "v4_grade.rs"]
mod grade;

/// A number field's value after an arrow key: the typed text (else the
/// field's value) one step up or down, ten with Shift, within its limits.
fn arrow_step(typed: &str, facts: crate::v4_compile::NumberFacts, key: &str) -> f64 {
    let current = typed
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .unwrap_or(facts.value);
    let step = if facts.step > 0. { facts.step } else { 1. };
    let delta = match key {
        "up" => step,
        "down" => -step,
        "shiftUp" => step * 10.,
        _ => -step * 10.,
    };
    let next = current + delta;
    // A step keeps to the step's own grid: 0.1 + 0.2 shows 0.3.
    let places = (-step.log10().floor()).max(0.) as i32 + 1;
    let scale = 10f64.powi(places);
    ((next * scale).round() / scale).clamp(facts.min, facts.max)
}

/// A press on a number field: a click edits, a drag scrubs (language.md §9).
/// A press on a grading control (`grade`) drags it through the same gesture.
struct ScrubDrag {
    id: Id,
    start: Point<Pixels>,
    number: crate::v4_compile::NumberFacts,
    binding: Option<SharedString>,
    active: bool,
    grade: Option<grade::GradeDrag>,
}
enum ScrubOp {
    Begin,
    Value(SharedString, Value),
    Finish(bool),
}
#[derive(Clone, Copy)]
struct Capture {
    target: Id,
    button: u64,
    /// Where the press was: an item that drags lifts once the pointer has
    /// moved past the drag threshold from here.
    origin: Point<Pixels>,
}
/// A scroller anchored at its end (`metadata.scrollAnchor: "end"`): the
/// panel hears when the reader scrolls away from the end and back (a
/// `scroll` event, `{away}`) so it can offer the way back, and a new
/// `metadata.scrollToken` takes it there.
#[derive(Default)]
struct Follow {
    token: Value,
    away: bool,
}
/// How far from the end still counts as at the end (the original panel's
/// jump threshold).
const FOLLOW_SLACK: f32 = 48.;
/// A publication compiled off the main thread, waiting to install.
struct Compiled {
    ticket: u64,
    publication: Publication,
    table: Arc<TokenTable>,
    prepared: Prepared,
}
pub struct SurfaceView {
    pub runtime: Arc<Builtins>,
    pub publication: Publication,
    table: Arc<TokenTable>,
    pub(crate) prepared: Rc<Prepared>,
    window: AnyWindowHandle,
    appkit: bool,
    natives: Natives,
    fields: BTreeMap<Id, Entity<TextField>>,
    controls: BTreeMap<Id, Entity<crate::v4_control::NativeControl>>,
    /// Pop-ups whose rows the host lists, by the source they were given.
    row_sources: HashMap<Id, (EntityId, Value)>,
    /// Disclosure buttons pressed whose group the script has not yet shown
    /// opened or closed: the chevron and the group's content follow the
    /// press in the next frame, the panel's own work after it.
    pub(crate) disclosures: HashMap<Id, bool>,
    scrolls: BTreeMap<Id, ScrollHandle>,
    leases: BTreeMap<Id, Option<SharedString>>,
    option_sets: BTreeMap<Id, Value>,
    focus: BTreeMap<Id, FocusHandle>,
    /// Hover washes (pm-design springs), stepped once per render.
    motion: pm_shell::motion::Motion<Id>,
    /// Floating surfaces marked `pmKit.presence` (the Light panel): how far
    /// in they are this frame, 0–1. Set before the motion frame steps.
    presence: HashMap<Id, f32>,
    subscriptions: BTreeMap<Id, Vec<Subscription>>,
    origin: Rc<Cell<Point<Pixels>>>,
    anchors: Rc<RefCell<HashMap<Id, Bounds<Pixels>>>>,
    /// The height a sheet's body wants, measured as it is laid out.
    sheet_fit: Rc<std::cell::Cell<f32>>,
    /// End-anchored scrollers' tokens and whether the reader is away, by node.
    follow: RefCell<HashMap<Id, Follow>>,
    /// While an agent tool draws (`agent::probe`), controls record where they
    /// were drawn so a panel point maps to the control under it.
    probing: bool,
    targets: Rc<RefCell<HashMap<Id, Bounds<Pixels>>>>,
    capture: Option<Capture>,
    scrub: Option<ScrubDrag>,
    scrub_ops: VecDeque<ScrubOp>,
    /// The panel item being dragged (its `dragstart` ran): `dragend` goes to
    /// it once the drag is over, wherever it was dropped.
    dragging: Option<Id>,
    scrub_gesture: Option<u64>,
    scrub_busy: bool,
    scrub_shown: Option<(Id, f64)>,
    grade_drawn: grade::Drawn,
    /// Each wheel's puck on its way to where its values put it.
    grade_motion: grade::Motion,
    move_busy: bool,
    /// Last label of each `role="status"` node; `None` until the first frame,
    /// so a panel's opening state is not read out.
    statuses: Option<HashMap<Id, String>>,
    queued_move: Option<(Id, EventDetail)>,
    /// Publications received, and the newest one compiled and shown.
    latest: u64,
    shown: u64,
    /// Events sent to the script and not yet handled; an agent's action
    /// waits for its own (`wake_idle` asks for a notify when they finish).
    handling: usize,
    pub(crate) wake_idle: bool,
    /// The newest compiled tree, kept when it finished while the view had no
    /// live window (a closed sheet); the next window installs it on attach.
    parked: Option<Compiled>,
    requested: Option<(usize, usize)>,
    /// A text layer whose Properties text to focus once its row shows
    /// (`inspector.focusText`), until the moment passes.
    focus_text: Option<(String, std::time::Instant)>,
    /// A semantic id to scroll into view once a publication has it.
    reveal: Option<(String, std::time::Instant)>,
    /// Told when a new tree is installed: the timeline's native timecode
    /// layer steps aside while its time field is open, and back after.
    pub(crate) installed: Option<async_channel::Sender<()>>,
    #[cfg(test)]
    pub renders: usize,
    #[cfg(test)]
    pub log: Vec<(Id, String, EventDetail)>,
    pub error: Option<String>,
}
impl SurfaceView {
    pub fn new(
        runtime: Arc<Builtins>,
        publication: Publication,
        tokens: pm_design::Tokens,
        natives: Natives,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        use raw_window_handle::HasWindowHandle;
        let table = Arc::new(TokenTable::new(tokens));
        let prepared = Rc::new(Prepared::new(publication.surface.clone(), &table));
        let mut this = Self {
            runtime,
            publication,
            table,
            prepared,
            window: (*window).window_handle(),
            appkit: HasWindowHandle::window_handle(window).is_ok(),
            natives,
            fields: BTreeMap::new(),
            controls: BTreeMap::new(),
            row_sources: HashMap::new(),
            disclosures: HashMap::new(),
            scrolls: BTreeMap::new(),
            leases: BTreeMap::new(),
            option_sets: BTreeMap::new(),
            focus: BTreeMap::new(),
            motion: Default::default(),
            presence: HashMap::new(),
            subscriptions: BTreeMap::new(),
            origin: Rc::new(Cell::new(Point::default())),
            anchors: Default::default(),
            sheet_fit: Default::default(),
            follow: Default::default(),
            probing: false,
            targets: Default::default(),
            capture: None,
            scrub: None,
            scrub_ops: VecDeque::new(),
            dragging: None,
            scrub_gesture: None,
            scrub_busy: false,
            scrub_shown: None,
            grade_drawn: Default::default(),
            grade_motion: Default::default(),
            move_busy: false,
            statuses: None,
            queued_move: None,
            latest: 0,
            shown: 0,
            handling: 0,
            wake_idle: false,
            parked: None,
            requested: None,
            focus_text: None,
            reveal: None,
            installed: None,
            #[cfg(test)]
            renders: 0,
            #[cfg(test)]
            log: vec![],
            error: None,
        };
        this.reconcile(window, cx);
        this
    }
    /// A reopened panel window keeps its view; later publications install
    /// there, and so does one that finished while no window was open.
    pub fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        if handle != self.window && (!self.controls.is_empty() || !self.fields.is_empty()) {
            // AppKit controls and editors live in the window they were
            // mounted in. A reopened window (the Agent popover closes and
            // opens again with the same view) mounts its own, or its
            // pop-ups, buttons and composer would be missing.
            // Their text carries over: once typed, a field's text lives in
            // the editor, not in the published value.
            let texts: Vec<(Id, String)> = self
                .fields
                .iter()
                .map(|(id, f)| (*id, f.read(cx).value().to_owned()))
                .collect();
            self.controls.clear();
            self.fields.clear();
            self.leases.clear();
            self.option_sets.clear();
            self.subscriptions.clear();
            cx.defer_in(window, move |v, window, cx| {
                v.reconcile(window, cx);
                for (id, text) in texts {
                    if let Some(field) = v.fields.get(&id) {
                        field.update(cx, |f, cx| f.set_value(&text, cx));
                    }
                }
                cx.notify();
            });
        }
        self.window = handle;
        if let Some(compiled) = self.parked.take() {
            self.install(compiled, window, cx);
        }
    }
    pub fn tokens(&self) -> &pm_design::Tokens {
        &self.table.tokens
    }
    /// How many publications this view has shown; it grows when the panel's
    /// tree changes.
    pub fn shown(&self) -> u64 {
        self.shown
    }
    /// A newer publication is still compiling.
    pub fn compiling(&self) -> bool {
        self.shown != self.latest
    }
    /// Compile off the main thread; only the newest publication installs.
    pub fn publish(&mut self, publication: Publication, cx: &mut Context<Self>) {
        if crate::diag::enabled() {
            crate::diag::line(
                serde_json::json!({"ui": "publication", "panel": publication.panel,
                "revision": publication.surface.revision(), "host": gpui_macos::feel::now()}),
            );
        }
        self.compile(publication, self.table.clone(), cx);
    }
    pub fn retheme(&mut self, tokens: pm_design::Tokens, cx: &mut Context<Self>) {
        let table = Arc::new(TokenTable::new(tokens));
        self.compile(self.publication.clone(), table, cx);
    }
    fn compile(
        &mut self,
        publication: Publication,
        table: Arc<TokenTable>,
        cx: &mut Context<Self>,
    ) {
        self.latest += 1;
        let ticket = self.latest;
        cx.spawn(async move |this, cx| {
            let surface = publication.surface.clone();
            let worker = table.clone();
            let prepared = cx
                .background_executor()
                .spawn(async move {
                    let started = std::time::Instant::now();
                    let prepared = Prepared::new(surface, &worker);
                    if crate::diag::enabled() {
                        crate::diag::line(serde_json::json!({"ui": "prepared",
                            "ms": started.elapsed().as_secs_f64() * 1000.}));
                    }
                    prepared
                })
                .await;
            let mut compiled = Some(Compiled {
                ticket,
                publication,
                table,
                prepared,
            });
            // The window the view is in now: a reopened sheet has a new one.
            let Ok(window) = this.update(cx, |v, _| v.window) else {
                return;
            };
            let installed = cx.update_window(window, |_, window, cx| {
                this.update(cx, |v, cx| {
                    if let Some(compiled) = compiled.take() {
                        v.install(compiled, window, cx);
                    }
                })
            });
            if installed.is_err() {
                let _ = this.update(cx, |v, _| {
                    if let Some(compiled) = compiled.take().filter(|c| c.ticket == v.latest) {
                        v.parked = Some(compiled);
                    }
                });
            }
        })
        .detach();
    }
    fn install(&mut self, compiled: Compiled, window: &mut Window, cx: &mut Context<Self>) {
        if compiled.ticket != self.latest {
            return;
        }
        let _span = crate::diag::span("panel.install");
        if crate::diag::enabled() {
            crate::diag::line(
                serde_json::json!({"ui": "install", "panel": compiled.publication.panel,
                "host": gpui_macos::feel::now()}),
            );
            // The frame that shows it reaches the glass: its own feel line.
            gpui_macos::feel::note_now("panel.install");
        }
        self.parked = None;
        self.shown = compiled.ticket;
        if self.publication.generation != compiled.publication.generation {
            self.restart();
        }
        self.table = compiled.table;
        self.publication = compiled.publication;
        self.prepared = Rc::new(compiled.prepared);
        self.settle_disclosures();
        {
            let _span = crate::diag::span("panel.reconcile");
            self.reconcile(window, cx);
        }
        self.try_focus_text(cx);
        self.try_reveal(cx);
        if let Some(installed) = &self.installed {
            let _ = installed.try_send(());
        }
        cx.notify();
    }
    /// Whether the node with this semantic id is an open text field (the
    /// timeline's time while someone types one), rather than its readout.
    pub fn field_open(&self, semantic: &str) -> bool {
        self.prepared
            .semantics
            .get(semantic)
            .and_then(|id| self.prepared.node(*id))
            .is_some_and(|node| node.cancel_change)
    }
    /// A new generation (an extension reloaded, a fork swapped in) reuses node
    /// ids, so nothing bound to the old tree may carry over.
    fn restart(&mut self) {
        self.disclosures.clear();
        self.controls.clear();
        self.fields.clear();
        self.scrolls.clear();
        self.leases.clear();
        self.option_sets.clear();
        self.focus.clear();
        self.subscriptions.clear();
        self.anchors.borrow_mut().clear();
        self.capture = None;
        self.scrub = None;
        self.queued_move = None;
        self.error = None;
    }
    fn reconcile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.announce_statuses(window);
        let prepared = self.prepared.clone();
        let live = |id: &Id| prepared.nodes.contains_key(id);
        self.controls.retain(|id, _| live(id));
        self.row_sources.retain(|id, _| live(id));
        self.scrolls.retain(|id, _| live(id));
        self.subscriptions.retain(|id, _| live(id));
        self.fields.retain(|id, _| live(id));
        self.focus.retain(|id, _| live(id));
        self.leases.retain(|id, _| live(id));
        self.option_sets.retain(|id, _| live(id));
        if self.capture.is_some_and(|c| !live(&c.target)) {
            self.capture = None;
        }
        let surface = self.publication.surface.clone();
        let mut mounts = (
            0usize,
            std::time::Duration::ZERO,
            0usize,
            std::time::Duration::ZERO,
        );
        for view in prepared.nodes.values() {
            let id = view.id;
            let Some(node) = surface.node(id) else {
                continue;
            };
            let lease = view.binding.clone();
            // A segmented control's choices are its segment children.
            let options = if view.component == Some(Component::SegmentedControl) {
                Value::Array(segments(&surface, node).0)
            } else {
                node.props.get("options").cloned().unwrap_or(Value::Null)
            };
            let leased = self.leases.get(&id) == Some(&lease);
            if leased
                && view.component == Some(Component::PopUpButton)
                && self.controls.contains_key(&id)
            {
                // A pop-up takes new rows in place (`NativeControl::update`):
                // the viewer's zoom names its Fit row after the current
                // zoom, which changes with every step of a window resize,
                // and a replaced control is gone until its first paint.
                self.option_sets.insert(id, options);
            } else if !leased || self.option_sets.get(&id) != Some(&options) {
                self.subscriptions.remove(&id);
                self.fields.remove(&id);
                self.controls.remove(&id);
                self.leases.insert(id, lease.clone());
                self.option_sets.insert(id, options);
            }
            if view.scroll_y || view.scroll_x {
                self.scrolls.entry(id).or_default();
            }
            let kind = self
                .appkit
                .then(|| control_kind(&self.publication.panel, node))
                .flatten();
            if let Some(kind) = kind {
                let started = std::time::Instant::now();
                let existed = self.controls.contains_key(&id);
                self.mount_control(kind, view, node, lease, window, cx);
                self.host_rows(id, node, cx);
                mounts.0 += 1;
                mounts.1 += started.elapsed();
                if crate::diag::enabled() && started.elapsed().as_secs_f64() > 0.0005 {
                    crate::diag::line(
                        serde_json::json!({"ui": "control", "popup": matches!(kind, crate::v4_control::Kind::Popup),
                        "new": !existed, "options": options_len(node),
                        "ms": started.elapsed().as_secs_f64() * 1000.}),
                    );
                }
                // A native control is a Tab stop too: the panel draws its
                // focus ring and forwards its keys (Full Keyboard Access).
                // A search field and a segmented control take no forwarded
                // keys (`NativeControl::key`), so Tab passes them.
                if !view.disabled
                    && view.tab_index.is_none_or(|t| t >= 0)
                    && !matches!(
                        kind,
                        crate::v4_control::Kind::Search | crate::v4_control::Kind::Segmented
                    )
                {
                    self.focus.entry(id).or_insert_with(|| cx.focus_handle());
                }
                continue;
            }
            if !view.disabled
                && (view.component.is_some_and(|c| c.interactive())
                    || view.tab_index.is_some_and(|v| v >= 0))
            {
                self.focus.entry(id).or_insert_with(|| cx.focus_handle());
            }
            if !self.appkit
                || !matches!(
                    view.component,
                    Some(Component::NumberField | Component::TextField)
                )
                || view.disabled
            {
                continue;
            }
            let started = std::time::Instant::now();
            self.mount_field(view, node, lease, window, cx);
            mounts.2 += 1;
            mounts.3 += started.elapsed();
        }
        if crate::diag::enabled() {
            crate::diag::line(
                serde_json::json!({"ui": "reconcile", "nodes": prepared.nodes.len(),
                "controls": mounts.0, "controls_ms": mounts.1.as_secs_f64() * 1000.,
                "fields": mounts.2, "fields_ms": mounts.3.as_secs_f64() * 1000.}),
            );
        }
    }
    /// Whether a disclosure button shows its group open: the press not yet
    /// published, else the publication.
    pub(crate) fn disclosure_open(&self, id: Id) -> bool {
        self.disclosures.get(&id).copied().unwrap_or_else(|| {
            self.publication
                .surface
                .node(id)
                .and_then(|n| n.props.get("aria-expanded"))
                == Some(&Value::Bool(true))
        })
    }
    /// A disclosure group's button: its heading, or the heading's first
    /// part when the heading carries an accessory.
    fn group_button(&self, group: &NodeView) -> Option<Id> {
        let first = self.prepared.node(*group.children.first()?)?;
        if first.component == Some(Component::DisclosureButton) {
            return Some(first.id);
        }
        first.children.iter().copied().find(|c| {
            self.prepared.node(*c).and_then(|n| n.component) == Some(Component::DisclosureButton)
        })
    }
    /// Presses the publication now shows (or whose button is gone) need no
    /// stand-in any more.
    fn settle_disclosures(&mut self) {
        let surface = self.publication.surface.clone();
        self.disclosures.retain(|id, open| {
            surface.node(*id).is_some_and(|n| {
                (n.props.get("aria-expanded") == Some(&Value::Bool(true))) != *open
            })
        });
    }
    /// A pop-up whose rows the host lists (`HostRows`, kit `rows`): they are
    /// read from the latest publication when its menu opens, on the main
    /// thread, with no call to the panel's script. Built-in and extension
    /// panels get the same.
    fn host_rows(&mut self, id: Id, node: &pm_surface::Node, cx: &mut Context<Self>) {
        let Some(control) = self.controls.get(&id) else {
            return;
        };
        let source = host_rows_source(node);
        let given = self.row_sources.get(&id);
        if given.map(|(c, s)| (*c, s)) == source.as_ref().map(|s| (control.entity_id(), s))
            || (given.is_none() && source.is_none())
        {
            return;
        }
        let entity = control.entity_id();
        let provider = source.clone().map(|source| {
            let published = self.natives.published.clone();
            Rc::new(move || {
                let started = std::time::Instant::now();
                let p = published.borrow().clone();
                let rows = p
                    .and_then(|p| crate::layer_commands::host_rows(&p, &source))
                    .unwrap_or_default();
                if crate::diag::enabled() {
                    crate::diag::line(serde_json::json!({"ui": "rows", "count": rows.len(),
                        "ms": started.elapsed().as_secs_f64() * 1000.}));
                }
                rows
            }) as crate::v4_control::RowSource
        });
        control.update(cx, |c, _| c.fill_rows(provider));
        match source {
            Some(source) => self.row_sources.insert(id, (entity, source)),
            None => self.row_sources.remove(&id),
        };
    }
    /// The rows a pop-up offers: the host's list when it has one (what its
    /// menu shows when opened), else the published options.
    pub(crate) fn offered_rows(&self, id: Id) -> Option<Value> {
        if let Some((_, source)) = self.row_sources.get(&id)
            && let Some(p) = self.natives.published.borrow().clone()
            && let Some(rows) = crate::layer_commands::host_rows(&p, source)
        {
            return Some(Value::Array(rows));
        }
        self.option_sets.get(&id).cloned()
    }
    /// `role="status"` is a live region: a status that appears or changes is
    /// posted as an NSAccessibility announcement (VoiceOver reads it).
    fn announce_statuses(&mut self, window: &Window) {
        let surface = self.publication.surface.clone();
        let current = self.prepared.nodes.keys().filter_map(|id| {
            let node = surface.node(*id)?;
            (node.props.get("role").and_then(Value::as_str) == Some("status")).then(|| {
                let label = node.props.get("label").and_then(Value::as_str);
                (*id, label.map_or_else(|| node.text.clone(), str::to_owned))
            })
        });
        let messages = status_announcements(&mut self.statuses, current);
        if messages.is_empty() || !self.appkit {
            return;
        }
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return;
        };
        let view: objc2::rc::Retained<objc2_app_kit::NSView> =
            unsafe { objc2::rc::Retained::retain(handle.ns_view.as_ptr().cast()).unwrap() };
        if let Some(ns_window) = view.window() {
            for message in messages {
                crate::v4_native::announce(&ns_window, &message);
            }
        }
    }
    fn mount_control(
        &mut self,
        kind: crate::v4_control::Kind,
        view: &NodeView,
        node: &pm_surface::Node,
        lease: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = view.id;
        let segmented = matches!(kind, crate::v4_control::Kind::Segmented)
            .then(|| segments(&self.publication.surface, node));
        let value = match &segmented {
            Some((_, selected, _)) => json!(selected),
            None => node.props.get("value").cloned().unwrap_or(Value::Null),
        };
        // A pop-up may title itself apart from its rows (`Fit (54 %)`).
        let display = node
            .props
            .get("metadata")
            .and_then(|m| m.get("pmKit"))
            .and_then(|m| m.get("display"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(control) = self.controls.get(&id) {
            control.update(cx, |v, _| v.display(display.as_deref()));
            if matches!(kind, crate::v4_control::Kind::Push)
                && let Some(label) = node.props.get("label").and_then(Value::as_str)
            {
                control.update(cx, |v, cx| v.retitle(label, cx));
            }
            let options = match &segmented {
                Some((options, _, _)) => options.clone(),
                None => control_options(kind, node),
            };
            control.update(cx, |v, cx| v.update(&value, &options, view.disabled, cx));
            // A filter the panel's host applied shows even mid-typing.
            if let Some(revision) = node
                .props
                .get("metadata")
                .and_then(|m| m.get("hostValue"))
                .and_then(Value::as_u64)
            {
                let text = value.as_str().unwrap_or_default().to_owned();
                control.update(cx, |v, _| v.host_text(&text, revision));
            }
            return;
        }
        let label = node
            .props
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("");
        let placeholder = node
            .props
            .get("metadata")
            .and_then(|m| m.get("placeholder"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let identifier = format!(
            "v4.{}.{}",
            self.publication.panel,
            node.props
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(label)
        );
        let options = match segmented {
            Some((options, _, _)) => options,
            None => control_options(kind, node),
        };
        let button_role = node
            .props
            .get("metadata")
            .and_then(|m| m.get("pmKit"))
            .and_then(|m| m.get("buttonRole"))
            .and_then(Value::as_str);
        let primary = button_role == Some("default")
            || node.props.get("attr").and_then(|a| a.get("default-button"))
                == Some(&Value::Bool(true))
            || node
                .props
                .get("metadata")
                .and_then(|m| m.get("export"))
                .and_then(|e| e.get("default"))
                == Some(&Value::Bool(true));
        let tokens = self.table.tokens.clone();
        let disabled = view.disabled;
        let control = cx.new(|cx| {
            crate::v4_control::NativeControl::new(
                crate::v4_control::ControlSpec {
                    kind,
                    label,
                    identifier: &identifier,
                    value: &value,
                    options,
                    disabled,
                    default: primary,
                    destructive: button_role == Some("destructive"),
                    // Bar pop-ups are borderless titles; the rest keep the
                    // bezel. `quiet` is the borderless title in the
                    // secondary ink, for an option bar under a composer.
                    inline: matches!(crate::v4_compile::variant(node), Some("inline" | "quiet"))
                        && matches!(kind, crate::v4_control::Kind::Popup),
                    well: view.well,
                    quiet: crate::v4_compile::variant(node) == Some("quiet"),
                    tint: None,
                    symbol: None,
                    // `data-pm-variant="glass"`: a segmented control in the
                    // system's capsule at the large size.
                    glass: crate::v4_compile::variant(node) == Some("glass")
                        && matches!(kind, crate::v4_control::Kind::Segmented),
                },
                &tokens,
                window,
                cx,
            )
        });
        let generation = self.publication.generation;
        self.subscriptions.entry(id).or_default().push(cx.subscribe(
            &control,
            move |v, _, value, cx| {
                if v.publication.generation != generation || v.leases.get(&id) != Some(&lease) {
                    return;
                }
                if matches!(kind, crate::v4_control::Kind::Push) {
                    v.send(id, "click", EventDetail::default(), cx);
                } else if matches!(kind, crate::v4_control::Kind::Segmented) {
                    // Choosing a segment is a press on that segment's node.
                    let segment = v.publication.surface.node(id).and_then(|node| {
                        let (_, _, ids) = segments(&v.publication.surface, node);
                        ids.get(usize::try_from(value.as_i64()?).ok()?).copied()
                    });
                    if let Some(segment) = segment {
                        v.send(segment, "click", EventDetail::default(), cx);
                    }
                } else if matches!(kind, crate::v4_control::Kind::Search) {
                    // The text as it is typed and the keys that leave the
                    // field, as a text field reports them.
                    let listens = |event: &str| {
                        v.publication
                            .surface
                            .node(id)
                            .is_some_and(|n| n.listeners.contains_key(event))
                    };
                    if let Some(key) = value.get("key").and_then(Value::as_str) {
                        if listens("keydown") {
                            let detail = EventDetail {
                                key: Some(key.into()),
                                ..Default::default()
                            };
                            v.send(id, "keydown", detail, cx);
                        }
                    } else if listens("input") {
                        let detail = EventDetail {
                            value: Some(value.clone()),
                            ..Default::default()
                        };
                        v.send(id, "input", detail, cx);
                    } else {
                        v.change(id, value.clone(), cx);
                    }
                } else {
                    v.change(id, value.clone(), cx);
                }
            },
        ));
        control.update(cx, |v, _| v.display(display.as_deref()));
        // A search field starts with its node's text at the host's revision
        // of it, so only a newer host value replaces what is typed: a first
        // update that came after the first keystroke emptied the field.
        if let Some(revision) = node
            .props
            .get("metadata")
            .and_then(|m| m.get("hostValue"))
            .and_then(Value::as_u64)
        {
            let text = value.as_str().unwrap_or_default().to_owned();
            control.update(cx, |v, _| v.host_text(&text, revision));
        }
        if let Some(placeholder) = placeholder {
            control.read(cx).placeholder(&placeholder);
        }
        self.controls.insert(id, control);
    }
    /// The ink a panel gave a field's value (the accent on a value an
    /// expression drives).
    fn ink(view: &NodeView) -> Option<u32> {
        view.style.text.color.map(|c| {
            let c = hsla_to_rgba(c);
            let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
            (byte(c.red) << 16) | (byte(c.green) << 8) | byte(c.blue)
        })
    }
    fn mount_field(
        &mut self,
        view: &NodeView,
        node: &pm_surface::Node,
        lease: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = view.id;
        // Number fields read at their display precision, as when scrubbed.
        let value = match (view.number, node.props.get("value")) {
            (Some(number), Some(value)) if value.is_number() => {
                number.text(value.as_f64().unwrap_or(number.value))
            }
            (_, value) => value.map(value_text).unwrap_or_default(),
        };
        let placeholder = node
            .props
            .get("metadata")
            .and_then(|m| m.get("placeholder"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let unit = view.unit.clone();
        if let Some(field) = self.fields.get(&id) {
            let tokens = self.table.tokens.clone();
            // A value the panel's host set (`metadata.hostValue`) replaces
            // the text even mid-edit; nothing else does.
            let host = node
                .props
                .get("metadata")
                .and_then(|m| m.get("hostValue"))
                .and_then(Value::as_u64);
            let ink = Self::ink(view);
            field.update(cx, |f, cx| {
                f.set_tokens(tokens);
                f.set_ink(ink);
                f.set_placeholder(&placeholder);
                f.set_unit(unit);
                match host {
                    Some(revision) => f.set_value_from_host(&value, revision, cx),
                    None => f.set_value(&value, cx),
                }
            });
            return;
        }
        let number = view.component == Some(Component::NumberField);
        let label = node
            .props
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("");
        let composer = node
            .props
            .get("metadata")
            .is_some_and(|m| m.get("submitOnReturn") == Some(&Value::Bool(true)));
        let kind = if number {
            TextKind::DenseNumber
        } else if node.r#type == "textarea" && composer {
            TextKind::Composer
        } else if node.r#type == "textarea" {
            TextKind::Multiline
        } else {
            TextKind::DenseText
        };
        let tokens = self.table.tokens.clone();
        let field = cx.new(|cx| {
            let mut field = TextField::new(kind, label, &value, tokens, window, cx);
            // A composer's panel draws the surface it is written on; a
            // colour's hex stands in its row's well.
            field.set_bare(kind == TextKind::Composer || view.bare);
            if view.code {
                field.set_code(pm_design::MONO_FONT);
            }
            field.set_ink(Self::ink(view));
            field.set_placeholder(&placeholder);
            field.set_unit(unit);
            field
        });
        self.subscriptions.entry(id).or_default().push(cx.subscribe_in(
            &field,
            window,
            move |this, _, event, window, cx| {
                // A multiline field hands Tab on: focus moves to the next
                // (or previous) control, as from any other field.
                match event {
                    TextEvent::Key("tab") => return window.focus_next(cx),
                    TextEvent::Key("backtab") => return window.focus_prev(cx),
                    _ => {}
                }
                if this.prepared.node(id).and_then(|n| n.binding.clone()) != lease {
                    return;
                }
                match event {
                    TextEvent::Changed(value) => {
                        if this
                            .publication
                            .surface
                            .node(id)
                            .is_some_and(|n| n.listeners.contains_key("input"))
                        {
                            this.send(
                                id,
                                "input",
                                EventDetail {
                                    value: Some(json!(value)),
                                    ..Default::default()
                                },
                                cx,
                            );
                        }
                    }
                    TextEvent::Committed(value) => {
                        let value = if number {
                            match value.parse::<f64>() {
                                Ok(n) if n.is_finite() => json!(n),
                                _ => {
                                    this.error = Some("Enter a finite number".into());
                                    cx.notify();
                                    return;
                                }
                            }
                        } else {
                            json!(value)
                        };
                        this.change(id, value, cx);
                    }
                    // Return in a composer: the panel sees the text as typed,
                    // then the key, as a keyboard would deliver them.
                    TextEvent::Submitted(value) => {
                        let listens = |event: &str| {
                            this.publication
                                .surface
                                .node(id)
                                .is_some_and(|n| n.listeners.contains_key(event))
                        };
                        let (input, keydown) = (listens("input"), listens("keydown"));
                        if input {
                            let detail = EventDetail {
                                value: Some(json!(value)),
                                ..Default::default()
                            };
                            this.send(id, "input", detail, cx);
                        }
                        if keydown {
                            let detail = EventDetail {
                                key: Some("Enter".into()),
                                ..Default::default()
                            };
                            this.send(id, "keydown", detail, cx);
                        }
                    }
                    TextEvent::Cancelled => {
                        if this.prepared.node(id).is_some_and(|n| n.cancel_change) {
                            this.change(id, json!(""), cx);
                        }
                    }
                    // Escape in a composer reaches its panel as the key
                    // (the Agent's shortcuts menu, then its popover).
                    TextEvent::Key("escape") => {
                        if this
                            .publication
                            .surface
                            .node(id)
                            .is_some_and(|n| n.listeners.contains_key("keydown"))
                        {
                            let detail = EventDetail {
                                key: Some("Escape".into()),
                                ..Default::default()
                            };
                            this.send(id, "keydown", detail, cx);
                        }
                    }
                    // Up and Down in a number field step its value (Shift:
                    // ten steps) and write it, as a spin field does.
                    TextEvent::Key(key @ ("up" | "down" | "shiftUp" | "shiftDown")) if number => {
                        let Some(facts) = this.prepared.node(id).and_then(|n| n.number) else {
                            return;
                        };
                        let Some(field) = this.fields.get(&id).cloned() else {
                            return;
                        };
                        let typed = field.read(cx).editing_text();
                        let next = arrow_step(&typed, facts, key);
                        let shown = facts.text(next);
                        field.update(cx, |f, cx| f.replace_text(&shown, cx));
                        this.change(id, json!(next), cx);
                    }
                    // Canvas editors only.
                    TextEvent::Selection { .. } | TextEvent::Key(_) | TextEvent::Blurred => {}
                }
            },
        ));
        if let Some(focus) = self.focus.get(&id) {
            let subscription = cx.on_focus(focus, window, move |v, _, cx| {
                if let Some(field) = v.fields.get(&id) {
                    field.update(cx, |f, cx| f.begin_editing(cx));
                }
            });
            self.subscriptions.entry(id).or_default().push(subscription);
        }
        self.fields.insert(id, field);
    }
    /// Ask JS for the row window that covers native scroll, when it left the
    /// published one. Rows already overscan a viewport on each side.
    pub fn outline_rows(&mut self, rows: (usize, usize), cx: &mut Context<Self>) {
        let Some(node) = self
            .prepared
            .nodes
            .values()
            .find(|n| n.outline_window.is_some())
        else {
            return;
        };
        let (id, (first, count)) = (node.id, node.outline_window.unwrap());
        if rows.1 > rows.0
            && (rows.0 < first || rows.1 > first + count)
            && self.requested != Some(rows)
        {
            self.requested = Some(rows);
            self.send(
                id,
                "rangechange",
                EventDetail {
                    range: Some(rows),
                    ..Default::default()
                },
                cx,
            );
        }
    }
    #[cfg(test)]
    pub fn change_semantic(&mut self, semantic: &str, value: Value, cx: &mut Context<Self>) {
        let id = self.prepared.semantics[semantic];
        self.change(id, value, cx);
    }
    #[cfg(test)]
    pub fn id_of_text(&self, text: &str) -> Option<Id> {
        let surface = &self.publication.surface;
        let parent = |n: &NodeView| n.parent;
        self.prepared
            .nodes
            .values()
            .find(|n| n.text.as_deref() == Some(text))
            .map(|n| {
                parent(n)
                    .filter(|p| surface.node(*p).is_some())
                    .unwrap_or(n.id)
            })
    }
    #[cfg(test)]
    pub fn semantic_ids(&self) -> Vec<String> {
        self.prepared.semantics.keys().cloned().collect()
    }
    /// Development builds: `event` on the node whose semantic id is, or ends
    /// with, `semantic` (`--open reviewPanel:settings:select:nav.general`).
    #[cfg(debug_assertions)]
    pub fn review_send(&mut self, semantic: &str, event: &str, cx: &mut Context<Self>) {
        // `semantic=<json>` carries a value (a pop-up's choice, a switch's state).
        // A link carries spaces and quotes percent-encoded (`="Make%20it%20so"`).
        let decode = |value: &str| {
            let bytes = value.as_bytes();
            let mut out = Vec::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                let hex = bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match (bytes[i], hex) {
                    (b'%', Some(byte)) => {
                        out.push(byte);
                        i += 3;
                    }
                    (byte, _) => {
                        out.push(byte);
                        i += 1;
                    }
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        };
        let (semantic, value) = match semantic.split_once('=') {
            Some((semantic, value)) => (semantic, serde_json::from_str(&decode(value)).ok()),
            None => (semantic, None),
        };
        let id = self
            .prepared
            .semantics
            .iter()
            .filter(|(key, _)| key.as_str() == semantic || key.ends_with(semantic))
            .min_by_key(|(key, _)| key.len())
            .map(|(_, id)| *id);
        // A value committed to a field goes where typing and Return send it
        // (a bound field writes through its lease, not a JS listener).
        if let (Some(id), "change", Some(value)) = (id, event, value.clone()) {
            return self.change(id, value, cx);
        }
        if let Some(id) = id {
            let detail = EventDetail {
                value,
                ..Default::default()
            };
            self.send(id, event, detail, cx);
        }
    }
    /// Development builds: the panel's longest scroller swept end to end for
    /// `seconds`, a step a frame, as a fast scroll moves it (so the cost of
    /// frames that move hosted controls can be measured without a pointer).
    #[cfg(debug_assertions)]
    pub fn review_scroll(&mut self, seconds: f32, cx: &mut Context<Self>) {
        let Some(scroll) = self
            .scrolls
            .values()
            .max_by(|a, b| f32::from(a.max_offset().y).total_cmp(&f32::from(b.max_offset().y)))
            .cloned()
        else {
            return;
        };
        let timer = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let started = std::time::Instant::now();
            loop {
                let t = started.elapsed().as_secs_f32();
                // A sweep down and back up every 0.8 s.
                let along = 1. - ((t / 0.4) % 2. - 1.).abs();
                let done = t >= seconds;
                let y = if done {
                    0.
                } else {
                    along * f32::from(scroll.max_offset().y)
                };
                scroll.set_offset(point(px(0.), px(-y)));
                if this.update(cx, |_, cx| cx.notify()).is_err() || done {
                    break;
                }
                timer.timer(std::time::Duration::from_micros(8_333)).await;
            }
        })
        .detach();
    }
    /// Development builds: the panel's longest scroller put `points` down
    /// its content, for captures of what lies below the fold.
    #[cfg(debug_assertions)]
    pub fn review_scroll_to(&mut self, points: f32, cx: &mut Context<Self>) {
        if let Some(scroll) = self
            .scrolls
            .values()
            .max_by(|a, b| f32::from(a.max_offset().y).total_cmp(&f32::from(b.max_offset().y)))
        {
            let y = points.clamp(0., f32::from(scroll.max_offset().y));
            scroll.set_offset(point(px(0.), px(-y)));
            cx.notify();
        }
    }
    #[cfg(test)]
    pub fn click_semantic(&mut self, semantic: &str, cx: &mut Context<Self>) {
        let id = self.prepared.semantics[semantic];
        self.send(id, "click", EventDetail::default(), cx);
    }
    #[cfg(test)]
    pub fn cancellable_field(&self) -> Option<Id> {
        self.prepared
            .nodes
            .values()
            .find(|n| n.cancel_change)
            .map(|n| n.id)
    }
    /// A scrub or grading drag is in progress (tests).
    #[cfg(test)]
    pub fn scrubbing(&self) -> bool {
        self.scrub.is_some()
    }
    #[cfg(test)]
    pub fn first_number(&self) -> Option<String> {
        let mut numbers: Vec<_> = self
            .prepared
            .nodes
            .values()
            .filter(|n| n.number.is_some() && n.binding.is_some() && !n.disabled)
            .collect();
        numbers.sort_by_key(|n| n.id);
        numbers.first().map(|n| {
            n.semantic
                .as_ref()
                .map_or_else(|| format!("n{}", n.id), |s| s.to_string())
        })
    }
    #[cfg(test)]
    pub fn semantic_id_starting(&self, prefix: &str) -> Option<String> {
        self.prepared
            .semantics
            .keys()
            .find(|k| k.starts_with(prefix))
            .cloned()
    }
    pub fn has_header(&self) -> bool {
        self.prepared.header.is_some()
    }
    /// Development builds: every scroll region of the view moved down by
    /// `by` points (`review/scroll`).
    #[cfg(debug_assertions)]
    pub fn review_scroll_by(&mut self, by: f32, cx: &mut Context<Self>) {
        for scroll in self.scrolls.values() {
            let offset = scroll.offset();
            let most = scroll.max_offset().y;
            scroll.set_offset(point(offset.x, (offset.y - px(by)).clamp(-most, px(0.))));
        }
        cx.notify();
    }
    /// Scroll `semantic` into view now, or when a publication within two
    /// seconds first has it (a section the panel is about to add or open).
    pub fn reveal_semantic(&mut self, semantic: String, cx: &mut Context<Self>) {
        self.reveal = Some((semantic, std::time::Instant::now()));
        self.try_reveal(cx);
    }
    fn try_reveal(&mut self, cx: &mut Context<Self>) {
        let Some((semantic, asked)) = self.reveal.clone() else {
            return;
        };
        if asked.elapsed() > std::time::Duration::from_secs(2) {
            self.reveal = None;
        } else if self.prepared.semantics.contains_key(&semantic) {
            self.reveal = None;
            self.scroll_semantic(&semantic, cx);
        }
    }
    pub fn scroll_semantic(&mut self, semantic: &str, cx: &mut Context<Self>) {
        if let Some(id) = self.prepared.semantics.get(semantic)
            && let Some(node) = self.prepared.node(*id)
            && let Some(parent) = node.parent.and_then(|id| self.prepared.node(id))
            && let Some(scroll) = self.scrolls.get(&parent.id)
            && let Some(index) = parent.children.iter().position(|id| *id == node.id)
        {
            scroll.scroll_to_top_of_item(index);
            cx.notify();
        }
    }
    /// v1's `inspector.focusText(layer)`: the layer's text field focused
    /// with its text selected, so typing replaces it. The selection reaches
    /// this panel a moment after it is asked for, so this waits (two seconds
    /// at most) for the text row bound to exactly that layer, never another's.
    pub fn focus_text(&mut self, layer: String, cx: &mut Context<Self>) {
        self.focus_text = Some((layer, std::time::Instant::now()));
        self.try_focus_text(cx);
    }
    fn try_focus_text(&mut self, cx: &mut Context<Self>) {
        if self
            .focus_text
            .as_ref()
            .is_some_and(|(_, asked)| asked.elapsed() > std::time::Duration::from_secs(2))
        {
            self.focus_text = None;
        }
        let Some(field) = self
            .focus_text_row()
            .and_then(|id| self.fields.get(&id).cloned())
        else {
            return;
        };
        self.focus_text = None;
        field.update(cx, |f, cx| f.begin_editing_selecting_all(cx));
    }
    /// The Properties text row bound to exactly the layer `focus_text` asked for.
    pub(crate) fn focus_text_row(&self) -> Option<Id> {
        let (layer, _) = self.focus_text.as_ref()?;
        let id = *self.prepared.semantics.get("value:content.text")?;
        let binding = self.prepared.node(id)?.binding.clone()?;
        let bound = self
            .runtime
            .bound_layers(&self.publication.panel, &binding)?;
        (*bound == [layer.clone()]).then_some(id)
    }
    pub fn focus_semantic(&self, semantic: &str, window: &mut Window, cx: &mut App) {
        if let Some(id) = self.prepared.semantics.get(semantic) {
            if let Some(field) = self.fields.get(id) {
                field.update(cx, |f, cx| f.begin_editing(cx));
            } else if let Some(control) = self.controls.get(id) {
                control.read(cx).focus();
            } else if let Some(focus) = self.focus.get(id) {
                focus.focus(window, cx);
            }
        }
    }
    /// Re-stage AppKit slots whose cached paint did not run this frame.
    pub fn keep_native_slots(&self, cx: &App) {
        for control in self.controls.values() {
            control.read(cx).keep();
        }
        for field in self.fields.values() {
            field.read(cx).keep();
        }
    }
    fn run(
        &mut self,
        ticket: pm_runtime_v8::Result<pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>>,
        cx: &mut Context<Self>,
    ) {
        match ticket {
            Ok(ticket) => {
                cx.spawn(async move |this, cx| {
                    if let Err(error) = ticket.await {
                        let _ = this.update(cx, |v, cx| {
                            v.error = Some(error.to_string());
                            cx.notify();
                        });
                    }
                })
                .detach();
            }
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }
    pub fn send(&mut self, id: Id, event: &str, detail: EventDetail, cx: &mut Context<Self>) {
        #[cfg(test)]
        self.log.push((id, event.into(), detail.clone()));
        let runtime = self.runtime.clone();
        let p = self.publication.clone();
        let event = event.to_owned();
        let semantic = self.prepared.node(id).and_then(|n| n.semantic.clone());
        let sent = crate::diag::enabled().then(|| {
            crate::diag::line(
                serde_json::json!({"ui": "send", "panel": p.panel, "event": event,
                "semantic": semantic, "host": gpui_macos::feel::now()}),
            );
            std::time::Instant::now()
        });
        self.handling += 1;
        // Native planning can read model state. Keep its locks off the main thread.
        cx.spawn(async move |this, cx| {
            let ticket = cx
                .background_executor()
                .spawn(async move {
                    runtime.activate(
                        &p.panel,
                        p.generation,
                        p.surface.revision(),
                        id,
                        semantic.as_deref(),
                        &event,
                        detail,
                    )
                })
                .await;
            let error = match ticket {
                Ok(ticket) => ticket.await.err().map(|e| e.to_string()),
                Err(error) => Some(error.to_string()),
            };
            let _ = this.update(cx, |v, cx| {
                if let Some(sent) = sent {
                    crate::diag::line(
                        serde_json::json!({"ui": "handled", "panel": v.publication.panel,
                        "ms": sent.elapsed().as_secs_f64() * 1000.}),
                    );
                }
                v.handling -= 1;
                if v.disclosures.contains_key(&id) {
                    // The script has the press. Its publication settles the
                    // stand-in; one that never changes the group (refused)
                    // lets the button show the panel's own state again.
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(2))
                            .await;
                        let _ = this.update(cx, |v, cx| {
                            if v.disclosures.remove(&id).is_some() {
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
                if let Some(error) = error {
                    v.error = Some(error);
                    cx.notify();
                } else if v.handling == 0 && std::mem::take(&mut v.wake_idle) {
                    cx.notify();
                }
            });
        })
        .detach();
    }
    /// Every event sent to the panel's script has been handled.
    pub fn idle(&self) -> bool {
        self.handling == 0
    }
    /// Pointer stream events address the current tree: a publication landing
    /// mid-drag must not lose the release. Moves coalesce to one in flight.
    fn pointer(
        &mut self,
        id: Id,
        event: &'static str,
        detail: EventDetail,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        self.log.push((id, event.into(), detail.clone()));
        if event == "pointermove" {
            if self.move_busy {
                self.queued_move = Some((id, detail));
                return;
            }
            self.move_busy = true;
        }
        let runtime = self.runtime.clone();
        let p = self.publication.clone();
        cx.spawn(async move |this, cx| {
            let ticket = cx
                .background_executor()
                .spawn(
                    async move { runtime.pointer_event(&p.panel, p.generation, id, event, detail) },
                )
                .await;
            let result = match ticket {
                Ok(ticket) => ticket.await.map(|_| ()),
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |v, cx| {
                if let Err(error) = result {
                    v.error = Some(error.to_string());
                }
                if event == "pointermove" {
                    v.move_busy = false;
                    if let Some((id, detail)) = v.queued_move.take() {
                        v.pointer(id, "pointermove", detail, cx);
                    }
                }
            });
        })
        .detach();
    }
    /// Where an anchor node was last drawn (window points).
    pub fn anchor_bounds(&self, id: Id) -> Option<Bounds<Pixels>> {
        self.anchors.borrow().get(&id).copied()
    }
    pub fn change(&mut self, id: Id, value: Value, cx: &mut Context<Self>) {
        let Some(node) = self.publication.surface.node(id) else {
            return;
        };
        if node.listeners.contains_key("change") {
            // A native text observation uses a string; native numeric bindings go
            // through the model lease, not a duplicate JS callback.
            self.send(
                id,
                "change",
                EventDetail {
                    value: Some(value),
                    ..Default::default()
                },
                cx,
            );
        } else if let Some(binding) = node.props.get("binding").and_then(Value::as_str) {
            let ticket = self.runtime.write(&self.publication.panel, binding, value);
            self.run(ticket, cx);
        }
    }
    fn activate(
        &mut self,
        id: Id,
        detail: EventDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.publication.surface.node(id).cloned() else {
            return;
        };
        if node.props.get("disabled") == Some(&Value::Bool(true)) {
            return;
        }
        match Component::of(&node) {
            Some(Component::DisclosureButton) => {
                let open = self.disclosure_open(id);
                self.disclosures.insert(id, !open);
                cx.notify();
                self.send(id, "click", detail, cx);
            }
            Some(Component::Checkbox | Component::Toggle | Component::Switch) => self.change(
                id,
                json!(
                    !node
                        .props
                        .get("value")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                ),
                cx,
            ),
            Some(Component::PopUpButton) if self.appkit => {
                let options = self
                    .offered_rows(id)
                    .and_then(|rows| rows.as_array().cloned())
                    .or_else(|| node.props.get("options").and_then(Value::as_array).cloned())
                    .unwrap_or_default();
                let owner = cx.entity().downgrade();
                crate::v4_native::menu(window, &options.clone(), cx, move |index, cx| {
                    if let Some(index) = index
                        && let Some(option) = options.get(index)
                    {
                        let value = option.get("v").unwrap_or(option).clone();
                        let _ = owner.update(cx, |v, cx| v.change(id, value, cx));
                    }
                });
            }
            Some(Component::ColourWell) => {
                let initial = node
                    .props
                    .get("value")
                    .and_then(Value::as_str)
                    .and_then(crate::sheets::hex_colour);
                let owner = cx.entity().downgrade();
                let binding = node.props.get("binding").cloned();
                crate::colour_panel::menu(window, initial, cx, move |colour, cx| {
                    let _ = owner.update(cx, |v, cx| {
                        // The panel can outlive the row: its lease must still be this one.
                        let current = v.publication.surface.node(id);
                        if current.and_then(|n| n.props.get("binding")) == binding.as_ref() {
                            v.change(id, json!(colour), cx);
                        }
                    });
                });
            }
            _ => self.send(id, "click", detail, cx),
        }
    }
    fn detail(
        &self,
        position: Point<Pixels>,
        button: Option<u64>,
        buttons: u64,
        clicks: usize,
        modifiers: Modifiers,
    ) -> EventDetail {
        let origin = self.origin.get();
        EventDetail {
            pointer_id: Some(POINTER),
            x: Some(f32::from(position.x - origin.x) as f64),
            y: Some(f32::from(position.y - origin.y) as f64),
            button,
            buttons: Some(buttons),
            click_count: Some(clicks as u64),
            ..modified(modifiers)
        }
    }
    fn press(&mut self, id: Id, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let button = button_code(event.button);
        self.capture = Some(Capture {
            target: id,
            button,
            origin: event.position,
        });
        let detail = self.detail(
            event.position,
            Some(button),
            1 << button,
            event.click_count,
            event.modifiers,
        );
        self.pointer(id, "pointerdown", detail, cx);
    }
    fn scrub_press(&mut self, id: Id, event: &MouseDownEvent, window: &mut Window) {
        let Some(node) = self.prepared.node(id) else {
            return;
        };
        let Some(number) = node.number else {
            return;
        };
        // A press must not focus (and so open) the text field before a drag decides.
        window.prevent_default();
        self.scrub = Some(ScrubDrag {
            id,
            start: event.position,
            number,
            binding: node.binding.clone().filter(|_| !node.local_change),
            active: false,
            grade: None,
        });
    }
    fn scrub_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.scrub.as_ref().is_some_and(|d| d.grade.is_some()) {
            return self.grade_move(event, cx);
        }
        let threshold = self.table.tokens.metrics.drag_threshold;
        let Some(drag) = &mut self.scrub else {
            return;
        };
        let delta = event.position - drag.start;
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        if !drag.active {
            if dx.hypot(dy) < threshold {
                return;
            }
            drag.active = true;
            if drag.binding.is_some() {
                self.scrub_ops.push_back(ScrubOp::Begin);
            }
        }
        let n = drag.number;
        let Ok(value) = pm_surface::binding::scrub(pm_surface::binding::Scrub {
            start: n.value,
            delta: (dx as f64, dy as f64),
            step: n.step,
            speed: n.speed,
            shift: event.modifiers.shift,
            alt: event.modifiers.alt,
            bounds: (n.min, n.max),
        }) else {
            return;
        };
        let id = drag.id;
        self.scrub_shown = Some((id, value));
        match drag.binding.clone() {
            Some(binding) => {
                self.scrub_ops
                    .push_back(ScrubOp::Value(binding, json!(value)));
                self.scrub_pump(cx);
            }
            None => self.change(id, json!(value), cx),
        }
        if let Some(field) = self.fields.get(&id) {
            let text = n.text(value);
            field.update(cx, |f, cx| f.set_value(&text, cx));
        }
        cx.notify();
    }
    fn scrub_release(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(drag) = self.scrub.take() else {
            return;
        };
        if drag.active {
            if drag.binding.is_some() {
                self.scrub_ops.push_back(ScrubOp::Finish(commit));
                self.scrub_pump(cx);
            }
            if drag.grade.is_some() {
                cx.notify();
            }
        } else if drag.grade.is_some() {
            // A press on a grading control that never moved changes nothing.
        } else if let Some(field) = self.fields.get(&drag.id) {
            field.update(cx, |f, cx| f.begin_editing(cx));
        }
    }
    /// Escape during a scrub restores the value through the kernel gesture.
    pub fn cancel_gesture(&mut self, cx: &mut Context<Self>) -> bool {
        if self.scrub.is_none() {
            return false;
        }
        self.scrub_release(false, cx);
        true
    }
    fn scrub_pump(&mut self, cx: &mut Context<Self>) {
        if self.scrub_busy {
            return;
        }
        let Some(mut op) = self.scrub_ops.pop_front() else {
            return;
        };
        // Only the newest value matters while the kernel is busy.
        while matches!(op, ScrubOp::Value(..))
            && matches!(self.scrub_ops.front(), Some(ScrubOp::Value(..)))
        {
            op = self.scrub_ops.pop_front().unwrap();
        }
        self.scrub_busy = true;
        let runtime = self.runtime.clone();
        let panel = self.publication.panel.clone();
        let gesture = self.scrub_gesture;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let pending = match (op, gesture) {
                        (ScrubOp::Begin, _) => runtime.scrub_begin(&panel),
                        (ScrubOp::Value(handle, value), Some(gesture)) => {
                            runtime.scrub_step(&panel, &handle, gesture, value)
                        }
                        (ScrubOp::Finish(commit), Some(gesture)) => {
                            runtime.scrub_finish(&panel, gesture, commit)
                        }
                        _ => return Err("Scrub gesture was refused".to_owned()),
                    }
                    .map_err(|e| e.to_string())?;
                    pending
                        .await
                        .ok_or("Kernel closed")?
                        .map_err(|e| format!("{e:?}"))
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.scrub_busy = false;
                match result {
                    Ok(ack) => match ack.reply {
                        pm_kernel::Reply::Gesture(id) => v.scrub_gesture = Some(id),
                        pm_kernel::Reply::Throw(error) => {
                            v.error = Some(error.to_string());
                        }
                        _ => {}
                    },
                    Err(error) => v.error = Some(error),
                }
                if v.scrub_ops.is_empty() && v.scrub.is_none() {
                    v.scrub_gesture = None;
                    v.scrub_shown = None;
                }
                v.scrub_pump(cx);
            });
        })
        .detach();
    }
    /// Capture-phase window listeners: a pressed pointer keeps its target until
    /// release, wherever it travels, like AppKit drag tracking.
    fn tracking(cx: &mut Context<Self>, origin: Rc<Cell<Point<Pixels>>>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        canvas(
            move |bounds, _, _| origin.set(bounds.origin),
            move |_, _, window, _| {
                let moved = weak.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let _ = moved.update(cx, |v, cx| {
                        // The button is up: a release that never reached this
                        // window ends the gesture here, and the window's other
                        // listeners see this move (owner feedback W4, W5).
                        if (event.pressed_button.is_none() || !button_held())
                            && (v.scrub.is_some() || v.capture.is_some())
                        {
                            v.release_pointer(event.position, 1, event.modifiers, cx);
                            return;
                        }
                        if v.scrub.is_some() {
                            cx.stop_propagation();
                            v.scrub_move(event, cx);
                            return;
                        }
                        let Some(capture) = v.capture else {
                            return;
                        };
                        // A press on an item that drags (`onDragStart`) gives
                        // the pointer up past the threshold: the move goes on
                        // to the item, which lifts it (GPUI's drag).
                        let threshold = v.table.tokens.metrics.drag_threshold;
                        if v.prepared
                            .node(capture.target)
                            .is_some_and(|n| n.events.drag_start)
                            && (event.position - capture.origin).magnitude() > threshold as f64
                        {
                            v.capture = None;
                            return;
                        }
                        cx.stop_propagation();
                        if v.prepared
                            .node(capture.target)
                            .is_some_and(|n| n.events.pointer_move)
                        {
                            let detail = v.detail(
                                event.position,
                                None,
                                1 << capture.button,
                                0,
                                event.modifiers,
                            );
                            v.pointer(capture.target, "pointermove", detail, cx);
                        }
                    });
                });
                // A new press while a gesture is still held (its release
                // never came) ends that gesture first, so the press does its
                // own work and the window's moves are no longer held.
                let pressed = weak.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let _ = pressed.update(cx, |v, cx| {
                        if v.scrub.is_some() || v.capture.is_some() {
                            v.release_pointer(event.position, 1, event.modifiers, cx);
                        }
                    });
                });
                let released = weak.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let _ = released.update(cx, |v, cx| {
                        v.release_pointer(event.position, event.click_count, event.modifiers, cx)
                    });
                });
            },
        )
        .absolute()
        .inset_0()
    }
    /// The pressed pointer comes up: a scrub or a grading drag ends (one
    /// Undo step), a captured node hears `pointerup`.
    fn release_pointer(
        &mut self,
        position: Point<Pixels>,
        click_count: usize,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        self.scrub_release(true, cx);
        let Some(capture) = self.capture.take() else {
            return;
        };
        if self
            .prepared
            .node(capture.target)
            .is_some_and(|n| n.events.pointer_up)
        {
            let detail = self.detail(position, Some(capture.button), 0, click_count, modifiers);
            self.pointer(capture.target, "pointerup", detail, cx);
        }
    }
    fn element(
        &self,
        id: Id,
        window: &mut Window,
        cx: &mut Context<Self>,
        skip: Option<Id>,
    ) -> AnyElement {
        if skip == Some(id) {
            return div().into_any_element();
        }
        let Some(node) = self.prepared.node(id) else {
            return div().into_any_element();
        };
        if node.hidden {
            return div().into_any_element();
        }
        if let Some(control) = self.controls.get(&id) {
            // A control whose node may shrink gives way (and truncates)
            // before its row pushes later controls out of the panel.
            let shrinks = node.style.flex_shrink.is_some_and(|s| s > 0.);
            let (fill, width) = control.read_with(cx, |c, _| (c.fill, c.width()));
            if fill != shrinks {
                control.update(cx, |c, _| c.fill = shrinks);
            }
            if node.style.flex_grow.is_some_and(|g| g > 0.) && !control.read(cx).stretches() {
                control.update(cx, |c, _| c.stretch());
            }
            let t = &self.table.tokens;
            let slot = if node.well && control.read(cx).is_popup() {
                // A pop-up in a row: the real control, with no bezel, on the
                // well every other value of the row stands in.
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .h(px(t.metrics.field))
                    .rounded(px(t.metrics.inner_radius))
                    .bg(crate::v4_compile::well(t))
                    .overflow_hidden()
            } else if control.read(cx).stretches() {
                // A segmented control its panel lets grow fills its row.
                div().w_full()
            } else if control.read(cx).is_search() {
                // A search field takes the room its row has.
                div().flex_1().min_w(px(0.))
            } else if shrinks {
                div()
                    .flex_basis(px(width))
                    .flex_shrink(1.)
                    .min_w(px(0.))
                    .overflow_hidden()
            } else {
                div().flex_none()
            };
            let mut slot = slot.id(("v4-control", id));
            if let Some(focus) = self.focus.get(&id) {
                let t = &self.table.tokens;
                let (ring, color, radius) = (
                    px(t.metrics.border * 2.),
                    rgb(t.colors.focus),
                    px(t.metrics.control_radius),
                );
                let tab = node.tab_index.unwrap_or(0);
                let handle = &focus.clone().tab_stop(tab >= 0).tab_index(tab.max(0));
                let control = control.clone();
                slot = slot
                    .track_focus(handle)
                    .tab_index(tab.max(0))
                    .rounded(radius)
                    .focus_visible_style(move |s| s.ring(ring).ring_color(color))
                    .on_key_down(move |event: &KeyDownEvent, _, cx| {
                        let m = &event.keystroke.modifiers;
                        if m.platform || m.control || m.alt {
                            return;
                        }
                        if control.read(cx).key(event.keystroke.key.as_str()) {
                            cx.stop_propagation();
                        }
                    });
            }
            // AppKit sets a bezel-less pop-up's title a little further in
            // than a field's value: bring it onto the wells' inset.
            let child = if node.well && control.read(cx).is_popup() {
                div()
                    .w_full()
                    .ml(px(-t.space.hair))
                    .child(control.clone())
                    .into_any_element()
            } else {
                control.clone().into_any_element()
            };
            return slot
                .child(child)
                .children(self.probe(id))
                .into_any_element();
        }
        if let Some(field) = self.fields.get(&id) {
            return self.field_element(node, field, cx);
        }
        let mut element = div().id(("v4", id));
        *element.style() = node.style.clone();
        if let Some(sample) = &node.fixed_width {
            let mut text = window.text_style();
            text.refine(&node.style.text);
            let size = text.font_size.to_pixels(window.rem_size());
            let run = text.to_run(sample.len());
            let width = window
                .text_system()
                .shape_line(sample.clone(), size, &[run])
                .width;
            element = element.min_w(width);
        }
        if node.outline_column && self.natives.outline.column.get() > 0. {
            element = element.w(px(self.natives.outline.column.get()));
        }
        if let Some(first) = &node.outline_first {
            element = element
                .absolute()
                .left(px(0.))
                .right(px(0.))
                .top(px(self.natives.outline.offset(first)));
        }
        #[cfg(test)]
        {
            let name = node
                .semantic
                .as_ref()
                .map_or_else(|| format!("n{id}"), |s| s.to_string());
            element = element.debug_selector(|| name);
        }
        if let Some(scroll) = self.scrolls.get(&id) {
            // A scroll region with a max height (a sheet body) keeps its
            // content height up to that cap; panel scrollers fill their dock.
            element = if node.style.max_size.height.is_some() {
                element.flex_grow(1.).flex_shrink(1.)
            } else {
                element.flex_1()
            };
            element = element.min_h(px(0.)).track_scroll(scroll);
            // `metadata.scrollAnchor: "end"` (a conversation): a scroller that
            // was at its end stays there as content grows; once scrolled up,
            // it stays where the person left it.
            let follows = self
                .publication
                .surface
                .node(id)
                .and_then(|n| n.props.get("metadata"))
                .and_then(|m| m.get("scrollAnchor"))
                .and_then(|a| a.as_str())
                == Some("end");
            if follows && scroll.offset().y <= -scroll.max_offset().y + px(1.) {
                scroll.scroll_to_bottom();
            }
            if node.scroll_y {
                element = element.overflow_y_scroll();
            }
            if follows {
                // `metadata.scrollToken`: a new value goes back to the end
                // (the panel's Scroll to Latest).
                let token = self
                    .publication
                    .surface
                    .node(id)
                    .and_then(|n| n.props.get("metadata"))
                    .and_then(|m| m.get("scrollToken"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let mut follow = self.follow.borrow_mut();
                let f = follow.entry(id).or_insert_with(|| Follow {
                    token: token.clone(),
                    away: false,
                });
                if f.token != token {
                    f.token = token;
                    scroll.scroll_to_bottom();
                }
                element = element.on_scroll_wheel(cx.listener(move |_, _, window, cx| {
                    cx.defer_in(window, move |v, _, cx| {
                        let Some(scroll) = v.scrolls.get(&id) else {
                            return;
                        };
                        let distance =
                            f32::from(scroll.max_offset().y) + f32::from(scroll.offset().y);
                        let away = distance > FOLLOW_SLACK;
                        let changed = {
                            let mut follow = v.follow.borrow_mut();
                            let f = follow.entry(id).or_default();
                            std::mem::replace(&mut f.away, away) != away
                        };
                        let listens = v
                            .publication
                            .surface
                            .node(id)
                            .is_some_and(|n| n.listeners.contains_key("scroll"));
                        if changed && listens {
                            v.send(
                                id,
                                "scroll",
                                EventDetail {
                                    value: Some(json!({ "away": away })),
                                    ..Default::default()
                                },
                                cx,
                            );
                        }
                    });
                }));
            }
            if node.scroll_x {
                element = element.overflow_x_scroll();
            }
        }
        if let Some(geometry) = node.virtual_window {
            element = element.on_scroll_wheel(cx.listener(move |_, _, window, cx| {
                cx.defer_in(window, move |v, _, cx| {
                    let Some(scroll) = v.scrolls.get(&id) else {
                        return;
                    };
                    let offset = -f32::from(scroll.offset().y) as f64;
                    let height = f32::from(scroll.bounds().size.height) as f64;
                    if let Ok(range) = geometry.visible(offset, height) {
                        v.send(
                            id,
                            "rangechange",
                            EventDetail {
                                range: Some(range),
                                ..Default::default()
                            },
                            cx,
                        );
                    }
                });
            }));
        }
        // A floating surface springs in when it appears and out while its
        // panel keeps it for the exit (`pmKit.closing`): opacity, and a
        // short rise from `motion.start_scale` worth of its height.
        if let Some(amount) = self.presence.get(&id).copied()
            && amount < 1.
        {
            let t = &self.table.tokens;
            let rise = (1. - amount) * (1. - t.motion.start_scale as f32) * 160.;
            element = element.opacity(amount).mt(px(rise));
        }
        element = self.accessible(element, node);
        if let Some(focus) = self.focus.get(&id) {
            let t = &self.table.tokens;
            let (ring, color) = (px(t.metrics.border * 2.), rgb(t.colors.focus));
            // A tracked handle carries its own tab settings (the element's
            // are ignored): `tabIndex` -1 stays focusable but out of the Tab
            // order, as on the web (a segmented control's other segments).
            let tab = node.tab_index.unwrap_or(0);
            let focus = &focus.clone().tab_stop(tab >= 0).tab_index(tab.max(0));
            element = element
                .track_focus(focus)
                .tab_index(tab.max(0))
                .focus_visible_style(move |s| s.ring(ring).ring_color(color));
        }
        element = self.interactive(element, node, cx);
        element = self.decorate(element, node, window);
        if let Some(drawing) = self.grade_canvas(id, &node.kit, node.disabled) {
            element = element.relative().child(drawing);
            if let Some(h) = self.grade_height(id, &node.kit) {
                element = element.h(px(h));
            }
            if !node.disabled {
                element = element.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, event: &MouseDownEvent, window, cx| {
                        window.prevent_default();
                        v.grade_press(id, event, cx);
                        cx.stop_propagation();
                    }),
                );
            }
        }
        let mut children: Vec<AnyElement> = vec![];
        // `data-pm-variant="plate"`: the grading material under a group (the
        // Colour panel's sections stand on what its wheels are made of).
        if crate::v4_compile::variant_of(&self.publication.surface, id) == Some("plate") {
            let tokens = self.table.tokens.clone();
            children.push(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        pm_shell::grade::paint_plate(bounds, &tokens, window)
                    },
                )
                .absolute()
                .inset_0()
                .into_any_element(),
            );
        }
        // A disabled field mounts no editor; it still shows its value, dimmed.
        if let Some(text) = self.disabled_field_text(node) {
            let t = &self.table.tokens;
            children.push(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(rgb(t.colors.muted))
                    .child(text)
                    .into_any_element(),
            );
        }
        let wash = self.motion.hovers.amount(id);
        if wash > 0. && !node.disabled {
            let t = &self.table.tokens;
            let alpha = (t.translucency.hover * wash * 255.).round() as u32;
            let mut layer = div().absolute().inset_0();
            layer.style().corner_radii = node.style.corner_radii.clone();
            children.push(
                layer
                    .bg(rgba(t.colors.text << 8 | alpha))
                    .into_any_element(),
            );
        }
        if let Some(geometry) = node.virtual_window {
            let (before, _) = geometry.spacers();
            children.push(div().flex_none().h(px(before as f32)).into_any_element());
        }
        // A group just closed by its button shows its heading alone now.
        let shown = if node.component == Some(Component::DisclosureGroup)
            && self
                .group_button(node)
                .and_then(|b| self.disclosures.get(&b))
                == Some(&false)
        {
            1
        } else {
            usize::MAX
        };
        for child in node.children.iter().take(shown) {
            match self.prepared.node(*child) {
                Some(c) if c.is_plain_text => {
                    if let Some(text) = &c.text {
                        children.push(text.clone().into_any_element());
                    }
                }
                _ => children.push(self.element(*child, window, cx, skip)),
            }
        }
        if let Some(geometry) = node.virtual_window {
            let (_, after) = geometry.spacers();
            children.push(div().flex_none().h(px(after as f32)).into_any_element());
        }
        if let Some(surface) = &node.native
            && let Some(view) = self.natives.views.borrow().get(surface.as_ref())
        {
            children.push(
                view.clone()
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element(),
            );
        }
        let mut element = element.children(children).children(self.probe(node.id));
        if self.fitted_sheet(node, window) {
            // The sheet is its window's height: its actions stay on the bottom
            // edge while the body between gives way.
            element = element.h(window.viewport_size().height);
        }
        let in_fitted_sheet = node
            .parent
            .and_then(|p| self.prepared.node(p))
            .is_some_and(|p| self.fitted_sheet(p, window));
        if node.sheet_body && in_fitted_sheet {
            return self.sheet_body(element.into_any_element(), id);
        }
        self.place(element.into_any_element(), node, window, cx)
    }
    /// A sheet shown in its own window (Export) is the whole window.
    fn own_sheet(&self, node: &NodeView, window: &Window) -> bool {
        node.component == Some(Component::SheetAnchor)
            && window.viewport_size().width <= px(self.table.tokens.metrics.sheet_width_wide + 1.)
    }
    /// An own-window sheet that names its body: the window follows the body's
    /// height on a spring instead of jumping to it.
    fn fitted_sheet(&self, node: &NodeView, window: &Window) -> bool {
        self.own_sheet(node, window)
            && node
                .children
                .iter()
                .any(|c| self.prepared.node(*c).is_some_and(|c| c.sheet_body))
    }
    /// The body of a sheet in its own window. It takes the room the window
    /// leaves between the title and the actions and clips what does not fit;
    /// the height it wants sets where the window is heading. The window grows
    /// and shrinks from its top edge on the layout spring, so nothing inside
    /// is moved by the animation but the actions riding the bottom edge.
    fn sheet_body(&self, element: AnyElement, id: Id) -> AnyElement {
        use pm_design::motion::{Animated, POINT_PRECISION, Preset, Spring};
        let t = &self.table.tokens;
        let spring = t.spring(Preset::Layout);
        // A capped scroll region and the rows that stand over it.
        let tallest = t.metrics.sheet_body_max + 2. * t.metrics.form_row;
        let (wanted, values) = (self.sheet_fit.clone(), self.motion.values.clone());
        let content = div().flex_none().relative().child(element).child(
            canvas(
                {
                    let wanted = wanted.clone();
                    move |bounds, _, _| wanted.set(f32::from(bounds.size.height))
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        );
        div()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
            .relative()
            .child(content)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        if wanted.get() < 1. {
                            return;
                        }
                        let viewport = f32::from(window.viewport_size().height);
                        let target =
                            (viewport - f32::from(bounds.size.height) + wanted.get()).round();
                        let height = match values.borrow_mut().entry(id) {
                            std::collections::hash_map::Entry::Occupied(mut value) => {
                                let value = value.get_mut();
                                if (value.target[0] - target).abs() > 0.5 {
                                    value.animate_to([target], spring);
                                    window.request_animation_frame();
                                }
                                value.get()
                            }
                            // The first fit is where the sheet opens.
                            std::collections::hash_map::Entry::Vacant(value) => {
                                value.insert(Animated::new(
                                    [target],
                                    Spring::SNAP,
                                    POINT_PRECISION,
                                ));
                                target
                            }
                        };
                        if (height - viewport).abs() > 0.5 {
                            // The most the sheet grows to: its body's cap.
                            let fixed = viewport - f32::from(bounds.size.height);
                            pm_shell::resize_from_top(window, height, fixed + tallest, cx);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }
    /// Whether a node's number is drawn with it (scrub values and wells with
    /// no editor). A disabled field draws its value once, dimmed, through
    /// [`Self::disabled_field_text`], so not here too.
    pub(crate) fn draws_number(&self, node: &NodeView) -> bool {
        node.number.is_some() && self.disabled_field_text(node).is_none()
    }
    /// What a disabled text or number field shows: its value as the editor
    /// would (number precision, then the unit).
    pub(crate) fn disabled_field_text(&self, node: &NodeView) -> Option<String> {
        if !node.disabled
            || !matches!(
                node.component,
                Some(Component::NumberField | Component::TextField)
            )
        {
            return None;
        }
        let props = &self.publication.surface.node(node.id)?.props;
        let value = props.get("value").filter(|v| !v.is_null())?;
        let text = match (node.number, value.as_f64()) {
            (Some(number), Some(n)) => number.text(n),
            _ => value_text(value),
        };
        // An empty one still says what it is for.
        if text.is_empty() {
            return props
                .get("metadata")
                .and_then(|m| m.get("placeholder"))
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        Some(match &node.unit {
            Some(unit) if !text.is_empty() => format!("{text} {unit}"),
            _ => text,
        })
    }
    /// Anchored kit surfaces escape the panel clip and stack above it.
    fn place(
        &self,
        element: AnyElement,
        node: &NodeView,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Kit::Anchored {
            anchor,
            point: at,
            dismiss,
        } = node.kit
        else {
            return element;
        };
        let t = &self.table.tokens;
        let origin = self.origin.get();
        // In its own sheet window (Export) the sheet is the whole window.
        let own_window = self.own_sheet(node, window);
        let position = if node.component == Some(Component::SheetAnchor) {
            let width = window.viewport_size().width;
            if own_window {
                point(px(0.), px(0.))
            } else {
                point(
                    (width - px(t.metrics.sheet_width)) / 2.,
                    px(t.metrics.toolbar),
                )
            }
        } else if let Some(bounds) = anchor.and_then(|id| self.anchors.borrow().get(&id).copied()) {
            bounds.bottom_left()
        } else if let Some((x, y)) = at {
            origin + point(px(x), px(y))
        } else {
            origin
        };
        let id = node.id;
        let mut wrapper = div().occlude().child(element);
        if own_window && !self.fitted_sheet(node, window) {
            // A sheet with no body of its own fits its window to its content.
            wrapper = wrapper.child(
                canvas(
                    |bounds, window, cx| {
                        let height = f32::from(bounds.size.height);
                        if height > 1.
                            && (window.viewport_size().height - bounds.size.height).abs() > px(0.5)
                        {
                            pm_shell::resize_from_top(window, height, height, cx);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        }
        if dismiss {
            wrapper = wrapper.on_mouse_down_out(cx.listener(move |v, _, _, cx| {
                v.send(
                    id,
                    "keydown",
                    EventDetail {
                        key: Some("Escape".into()),
                        ..Default::default()
                    },
                    cx,
                );
            }));
        }
        deferred(
            anchored()
                .position(position)
                .snap_to_window()
                .child(wrapper),
        )
        .priority(1)
        .into_any_element()
    }
    fn accessible(&self, mut element: Stateful<Div>, node: &NodeView) -> Stateful<Div> {
        if let Some(ax) = &node.ax {
            if let Some(role) = ax.role {
                element = element.role(role);
            }
            if let Some(label) = &ax.label {
                element = element.aria_label(label.clone());
            }
            if let Some(value) = &ax.value {
                element = element.aria_value(value.clone());
            }
            if ax.disabled {
                element = element.aria_disabled(true);
            }
            if let Some(selected) = ax.selected {
                element = element.aria_selected(selected);
            }
            if let Some(expanded) = ax.expanded {
                element = element.aria_expanded(expanded);
            }
            if let Some(toggled) = ax.toggled {
                element = element.aria_toggled(toggled);
            }
        }
        if let Some(semantic) = &node.semantic {
            element =
                element.accessibility_id(format!("v4.{}.{}", self.publication.panel, semantic));
        }
        if let Some(count) = node.row_count {
            element = element.aria_row_count(count);
        }
        element
    }
    fn interactive(
        &self,
        mut element: Stateful<Div>,
        node: &NodeView,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let id = node.id;
        let events = node.events;
        let disabled = node.disabled;
        let clickable = events.click
            || matches!(
                node.component,
                Some(
                    Component::PopUpButton
                        | Component::Checkbox
                        | Component::Toggle
                        | Component::ColourWell
                )
            );
        if !disabled && (clickable || events.double_click) {
            element = element.on_click(cx.listener(move |v, event: &ClickEvent, window, cx| {
                // A node that only takes double-clicks (a layer's name) leaves
                // a single click to what it sits in (its row selects).
                let double = events.double_click && event.click_count() == 2;
                if !clickable && !double {
                    return;
                }
                cx.stop_propagation();
                let position = event.position();
                let detail = v.detail(position, Some(0), 0, event.click_count(), event.modifiers());
                if clickable {
                    v.activate(id, detail.clone(), window, cx);
                }
                if events.double_click && event.click_count() == 2 {
                    v.send(id, "doubleclick", detail, cx);
                }
            }));
        }
        if disabled {
            return element;
        }
        // Finder drops reach targets that take files; the window's drop
        // handler places anything landing elsewhere at the playhead. The
        // panel's own items dragged onto them move there (into a bin).
        if events.file_drop {
            let target = rgb(self.table.tokens.colors.selected_fill);
            let panel = self.publication.panel.clone();
            let own = move |drag: &PanelDrag| drag.panel == panel;
            let own_over = own.clone();
            element = element
                .on_drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(target))
                .on_drop(cx.listener(move |v, paths: &ExternalPaths, window, cx| {
                    cx.stop_propagation();
                    let detail = file_drop(paths.paths(), window.modifiers().alt);
                    v.send(id, "drop", detail, cx);
                }))
                .on_drag_over::<PanelDrag>(move |style, drag, _, _| {
                    if own_over(drag) {
                        style.bg(target)
                    } else {
                        style
                    }
                })
                .on_drop(cx.listener(move |v, drag: &PanelDrag, window, cx| {
                    if !own(drag) {
                        return;
                    }
                    cx.stop_propagation();
                    let detail = EventDetail {
                        alt_key: Some(window.modifiers().alt),
                        ..Default::default()
                    };
                    v.send(id, "dragover", detail.clone(), cx);
                    v.send(id, "drop", detail, cx);
                    v.end_drag(cx);
                }));
        }
        // A panel item lifts after the drag threshold: the panel learns of
        // it (`dragstart`), and the timeline, the viewer and the panel's own
        // targets take it.
        if events.drag_start {
            let c = &self.table.tokens.colors;
            let drag = PanelDrag {
                panel: self.publication.panel.clone(),
                label: node
                    .ax
                    .as_ref()
                    .and_then(|ax| ax.label.clone())
                    .unwrap_or_else(|| "Media".into()),
                colors: (c.floating, c.separator, c.text),
            };
            let view = cx.entity().downgrade();
            element = element.on_drag(drag, move |drag, _, _, cx| {
                let view = view.clone();
                cx.defer(move |cx| {
                    let _ = view.update(cx, |v, cx| {
                        v.dragging = Some(id);
                        v.send(id, "dragstart", EventDetail::default(), cx);
                    });
                });
                cx.new(|_| drag.clone())
            });
        }
        if node.number.is_some() {
            element = element.cursor_col_resize().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, event: &MouseDownEvent, window, cx| {
                    v.scrub_press(id, event, window);
                    cx.stop_propagation();
                }),
            );
        }
        if let Some((layer, path)) = node.timeline_scrub.clone() {
            let outline = self.natives.outline.clone();
            element = element.cursor_col_resize().on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, _, cx| {
                    if outline.scrub_property(&layer, &path, event.position, cx) {
                        cx.stop_propagation();
                    }
                },
            );
        }
        if events.pointer() {
            element = element.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, event: &MouseDownEvent, _, cx| {
                    v.press(id, event, cx);
                    cx.stop_propagation();
                }),
            );
        }
        // Panels open their menus from a secondary press, the way they did in Electron.
        if events.pointer_down {
            element = element.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |v, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let detail = v.detail(event.position, Some(2), 2, 1, event.modifiers);
                    v.pointer(id, "pointerdown", detail, cx);
                }),
            );
        }
        if events.context_menu {
            element = element.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |v, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    let detail = v.detail(event.position, Some(2), 4, 1, event.modifiers);
                    v.send(id, "contextmenu", detail, cx);
                }),
            );
        }
        // One hover handler: the control's wash (a spring, never a snap) and
        // the panel's own pointer events. Only a control washes: a container
        // that listens to clicks to keep them from its list (a row of
        // Effect Controls) is not one.
        // A selectable item (a media row is a `cell` with `aria-selected`)
        // or a control by role washes; a plain container does not.
        let props = self.publication.surface.node(id).map(|n| &n.props);
        let control_role = props.is_some_and(|p| {
            p.contains_key("aria-selected")
                || p.get("role").and_then(Value::as_str).is_some_and(|r| {
                    matches!(
                        r,
                        "button"
                            | "link"
                            | "tab"
                            | "option"
                            | "menuitem"
                            | "cell"
                            | "gridcell"
                            | "row"
                            | "listitem"
                            | "treeitem"
                    )
                })
        });
        let wash = clickable && (node.component.is_some() || control_role);
        if wash || events.hover {
            element = element.on_hover(cx.listener(move |v, hovered: &bool, _, cx| {
                if wash {
                    let tokens = v.table.tokens.clone();
                    v.motion.hovers.hover(id, *hovered, &tokens);
                    cx.notify();
                }
                if events.hover {
                    let event = if *hovered {
                        "pointerenter"
                    } else {
                        "pointerleave"
                    };
                    v.pointer(id, event, EventDetail::default(), cx);
                }
            }));
        }
        if events.wheel {
            element = element.on_scroll_wheel(cx.listener(
                move |v, event: &ScrollWheelEvent, window, cx| {
                    cx.stop_propagation();
                    let delta = event.delta.pixel_delta(window.line_height());
                    let detail = EventDetail {
                        delta_x: Some(-f32::from(delta.x) as f64),
                        delta_y: Some(-f32::from(delta.y) as f64),
                        ..v.detail(event.position, None, 0, 0, event.modifiers)
                    };
                    v.pointer(id, "wheel", detail, cx);
                },
            ));
            // Browser pinch convention: a control-modified wheel, so one handler zooms.
            element = element.on_pinch(cx.listener(move |v, event: &PinchEvent, _, cx| {
                cx.stop_propagation();
                let detail = EventDetail {
                    delta_y: Some(-f64::from(event.delta) * 100.),
                    ctrl_key: Some(true),
                    ..v.detail(event.position, None, 0, 0, event.modifiers)
                };
                v.pointer(id, "wheel", detail, cx);
            }));
        }
        // Keys reach the panel's root and its header slot's root: the shell
        // draws a header slot (the viewer's bar) apart from the body, so the
        // body's root isn't an ancestor of the controls there.
        let slot_root = self
            .publication
            .surface
            .nodes()
            .get(&id)
            .is_some_and(|n| n.props.get("data-pm-slot").is_some());
        if (node.parent == Some(0) || slot_root) && self.prepared.has_keydown {
            element =
                element.on_key_down(cx.listener(move |v, event: &KeyDownEvent, window, cx| {
                    v.key_down(id, event, window, cx);
                }));
        }
        // A tooltip says what is not already in view: one that repeats the
        // text the element shows (an effect's name, a row's label) only
        // covers its neighbours.
        if let Some(title) = self.tooltip_title(node.id) {
            let tokens = self.table.tokens.clone();
            element = element.tooltip(move |_, cx| {
                let (title, tokens) = (title.clone(), tokens.clone());
                cx.new(|_| Tooltip { title, tokens }).into()
            });
        }
        element
    }
    /// The node holding keyboard focus (tests).
    #[cfg(test)]
    pub(crate) fn focused_node(&self, window: &Window) -> Option<Id> {
        self.focus
            .iter()
            .find(|(_, h)| h.is_focused(window))
            .map(|(id, _)| *id)
    }
    /// A key its window's root took while nothing in the panel was focused
    /// (a sheet just opened): the panel's root hears it as its own keys,
    /// so Return presses a sheet's default button and Escape cancels it.
    pub(crate) fn root_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self
            .publication
            .surface
            .node(0)
            .and_then(|n| n.children.first().copied());
        if let Some(root) = root
            && self.prepared.has_keydown
        {
            self.key_down(root, event, window, cx);
        }
    }
    fn key_down(
        &mut self,
        root: Id,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = match event.keystroke.key.as_str() {
            "up" => "ArrowUp",
            "down" => "ArrowDown",
            "left" => "ArrowLeft",
            "right" => "ArrowRight",
            "enter" => "Enter",
            "escape" => "Escape",
            "space" => " ",
            other => other,
        };
        let id = self
            .focus
            .iter()
            .find(|(_, h)| h.is_focused(window))
            .map(|(id, _)| *id)
            .unwrap_or(root);
        if pm_surface::interaction::Interaction::default()
            .plan(&self.publication.surface, id, None, "keydown", Some(key))
            .is_ok_and(|p| p.prevent_default)
        {
            window.prevent_default();
            cx.stop_propagation();
        }
        self.send(
            id,
            "keydown",
            EventDetail {
                key: Some(key.into()),
                repeat: Some(event.is_held),
                ..modified(event.keystroke.modifiers)
            },
            cx,
        );
    }
    /// Kit furniture that is not expressible as a generic style.
    fn decorate(
        &self,
        mut element: Stateful<Div>,
        node: &NodeView,
        window: &Window,
    ) -> Stateful<Div> {
        let t = &self.table.tokens;
        if self.prepared.anchors.contains(&node.id) {
            let (id, anchors) = (node.id, self.anchors.clone());
            element = element.child(
                canvas(
                    move |bounds, _, _| {
                        anchors.borrow_mut().insert(id, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            );
        }
        match &node.kit {
            Kit::Check(on) => {
                element = element.child(pm_shell::symbol::symbol(
                    if *on {
                        "checkmark.square.fill"
                    } else {
                        "square"
                    },
                    t.metrics.symbol_panel,
                    pm_shell::symbol::Weight::Regular,
                    t.colors.secondary,
                    window,
                ));
            }
            // The chip inside its well: the colour under a keyline drawn
            // inside its edge, so white reads on a light well and black on a
            // dark one.
            Kit::Swatch { color, keyline } => {
                let side = px(t.metrics.field - 2. * t.space.xs);
                let mut chip = div()
                    .size(side)
                    .rounded(px(t.radius.xs))
                    .shadow(vec![BoxShadow {
                        color: pm_shell::kit::ink(*keyline, 1.).into(),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(t.metrics.border),
                        inset: true,
                    }]);
                if let Some(color) = color {
                    chip = chip.bg(rgb(*color));
                }
                element = element.child(chip);
            }
            Kit::Progress(None) => {
                let tokens = t.clone();
                element = element.child(if t.accessibility.reduce_motion {
                    pm_shell::welcome_view::spinner(0., &tokens).into_any_element()
                } else {
                    div()
                        .flex_none()
                        .with_animation(
                            "progress",
                            Animation::new(std::time::Duration::from_secs(1)).repeat(),
                            move |this, turn| {
                                this.child(pm_shell::welcome_view::spinner(turn as f64, &tokens))
                            },
                        )
                        .into_any_element()
                });
            }
            Kit::Progress(Some(value)) => {
                let track = div()
                    .h(px(t.metrics.inner_radius))
                    .w_full()
                    .rounded(px(t.metrics.inner_radius / 2.))
                    .overflow_hidden()
                    .bg(rgb(t.colors.field))
                    .child(div().h_full().w(relative(*value)).bg(rgb(t.colors.accent)));
                element = element.child(track);
            }
            Kit::Gizmo {
                position,
                size,
                fill,
                stroke,
                keyline,
                round,
            } => {
                let half = size / 2.;
                element = element
                    .absolute()
                    .left(px(position.0 - half))
                    .top(px(position.1 - half))
                    .size(px(*size))
                    .bg(rgb(*fill))
                    .border(px(*keyline))
                    .border_color(rgb(*stroke));
                if *round {
                    element = element.rounded_full();
                }
            }
            Kit::Banner { symbol } => {
                if let Some((name, size, color)) = symbol {
                    element = element.child(pm_shell::symbol::symbol(
                        name,
                        *size,
                        pm_shell::symbol::Weight::Regular,
                        *color,
                        window,
                    ));
                }
            }
            // Drawn with their own canvas (`grade_canvas`).
            Kit::Wheel { .. } | Kit::Curve { .. } => {}
            Kit::Disclosure(published) => {
                let open = &self
                    .disclosures
                    .get(&node.id)
                    .copied()
                    .unwrap_or(*published);
                let group: SharedString = format!("disclosure-{}", node.id).into();
                element = element.group(group.clone()).child(
                    div()
                        .when(*open, |d| {
                            d.opacity(0.).group_hover_style(group, |s| s.opacity(1.))
                        })
                        .child(pm_shell::symbol::symbol(
                            if *open {
                                "chevron.down"
                            } else {
                                "chevron.right"
                            },
                            t.metrics.symbol_inline,
                            pm_shell::symbol::Weight::Regular,
                            t.colors.muted,
                            window,
                        )),
                );
            }
            Kit::Secure | Kit::Anchored { .. } | Kit::None => {}
        }
        if let Some(symbol) = &node.symbol {
            // The glyph takes the colour the panel declared (an active toggle's
            // accent, a keyframe state); disabled is always `muted`.
            let ink = if node.disabled {
                t.colors.muted
            } else {
                node.style.text.color.map_or(t.colors.secondary, |c| {
                    let c = hsla_to_rgba(c);
                    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
                    (byte(c.red) << 16) | (byte(c.green) << 8) | byte(c.blue)
                })
            };
            element = element.child(pm_shell::symbol::symbol(
                symbol,
                node.symbol_size,
                pm_shell::symbol::Weight::Regular,
                ink,
                window,
            ));
        }
        if let Some(text) = &node.text {
            element = element.child(text.clone());
        }
        if let Some(rich) = &node.rich {
            element = element.child(rich_text(rich, t));
        }
        if let Some(number) = node.number.filter(|_| self.draws_number(node)) {
            let shown = self
                .scrub_shown
                .filter(|(id, _)| *id == node.id)
                .map_or(number.value, |(_, value)| value);
            element = element.child(number.text(shown));
            if let Some(unit) = &node.unit {
                element = element.child(
                    div()
                        .ml(px(t.space.hair))
                        .text_color(rgb(t.colors.muted))
                        .child(unit.clone()),
                );
            }
        }
        element
    }
    fn field_element(
        &self,
        node: &NodeView,
        field: &Entity<TextField>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = node.id;
        // A field grows along a row (value wells); in a column it spans the
        // width at its own height instead of stretching downwards.
        let in_row = node
            .parent
            .and_then(|p| self.prepared.node(p))
            .is_some_and(|p| p.style.flex_direction == Some(FlexDirection::Row));
        // A field given a width keeps it: it has no title to be sized by,
        // so in a row as wide as its content (a form row's control) it would
        // otherwise be its padding wide.
        let own = node.style.size.width.filter(|_| in_row);
        let mut wrapper = div()
            .id(("field", id))
            .when_some(own, |d, width| d.flex_none().w(width))
            .when(in_row && own.is_none(), |d| d.flex_1())
            .when(!in_row, |d| d.flex_none().w_full())
            .min_w(px(0.))
            .child(field.clone())
            .children(self.probe(id));
        if let Some(ax) = &node.ax {
            if let Some(role) = ax.role {
                wrapper = wrapper.role(role);
            }
            wrapper = wrapper.aria_label(ax.label.clone().unwrap_or_default());
            if let Some(value) = &ax.value {
                wrapper = wrapper.aria_value(value.clone());
            }
        }
        if let Some(focus) = self.focus.get(&id) {
            let focus = &focus.clone().tab_stop(true).tab_index(0);
            wrapper = wrapper.track_focus(focus).tab_index(0);
        }
        if node.number.is_some() {
            wrapper = wrapper
                .cursor_col_resize()
                .capture_any_mouse_down(cx.listener(
                    move |v, event: &MouseDownEvent, window, cx| {
                        if event.button == MouseButton::Left {
                            v.scrub_press(id, event, window);
                            cx.stop_propagation();
                        }
                    },
                ));
        }
        let weak = cx.entity().downgrade();
        wrapper = wrapper.on_a11y_action(AccessibleAction::SetValue, move |data, _, cx| {
            let value = match data {
                Some(accesskit::ActionData::NumericValue(n)) if n.is_finite() => json!(n),
                Some(accesskit::ActionData::Value(text)) => json!(text.to_string()),
                _ => return,
            };
            let _ = weak.update(cx, |v, cx| v.change(id, value, cx));
        });
        if let Some(number) = node.number {
            wrapper = wrapper
                .aria_numeric_value(number.value)
                .aria_numeric_value_step(number.step);
            for (action, direction) in [
                (AccessibleAction::Increment, 1.),
                (AccessibleAction::Decrement, -1.),
            ] {
                let weak = cx.entity().downgrade();
                wrapper = wrapper.on_a11y_action(action, move |_, _, cx| {
                    let _ = weak.update(cx, |v, cx| {
                        if let Some(n) = v.prepared.node(id).and_then(|n| n.number) {
                            let next = (n.value + n.step * direction).clamp(n.min, n.max);
                            v.change(id, json!(next), cx);
                        }
                    });
                });
            }
        }
        let field = field.clone();
        wrapper = wrapper.on_a11y_action(AccessibleAction::Focus, move |_, _, cx| {
            field.update(cx, |f, cx| f.begin_editing(cx));
        });
        wrapper.into_any_element()
    }
}
/// A panel item being dragged (`onDragStart`), and the image under the
/// pointer while it is: its name on the floating fill.
#[derive(Clone, Debug)]
pub struct PanelDrag {
    /// The panel it came from (`Publication::panel`).
    pub panel: String,
    pub label: SharedString,
    /// Floating fill, its edge, text.
    colors: (u32, u32, u32),
}
impl Render for PanelDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (fill, edge, text) = self.colors;
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(rgb(fill))
            .border(px(1.))
            .border_color(rgb(edge))
            .text_color(rgb(text))
            .text_size(px(12.))
            .whitespace_nowrap()
            .child(self.label.clone())
    }
}
impl SurfaceView {
    /// The item drag is over (dropped anywhere, or let go): the panel
    /// clears its drag state.
    pub fn end_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.dragging.take() {
            self.send(id, "dragend", EventDetail::default(), cx);
        }
    }
}
/// A file drop as the panel's JS reads it (`public.file-url`, as on AppKit).
fn file_drop(paths: &[std::path::PathBuf], alt: bool) -> EventDetail {
    const FILES: &str = "public.file-url";
    let files = paths
        .iter()
        .map(|path| json!(path.to_string_lossy()))
        .collect();
    EventDetail {
        alt_key: Some(alt),
        drag: Some(DragData {
            types: vec![FILES.into()],
            data: BTreeMap::from([(FILES.to_owned(), Value::Array(files))]),
            effect: DragEffect::Copy,
        }),
        ..Default::default()
    }
}

impl Render for SurfaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A drag let go over nothing that took it ends here.
        if self.dragging.is_some() && !cx.has_active_drag() {
            let view = cx.entity().downgrade();
            cx.defer(move |cx| {
                let _ = view.update(cx, |v, cx| v.end_drag(cx));
            });
        }
        #[cfg(test)]
        {
            self.renders += 1;
        }
        let _span = crate::diag::span("panel.render");
        // Presences take their targets before the frame steps, so a surface
        // that just appeared or was marked closing starts moving now.
        let surface = self.publication.surface.clone();
        let mut presence = HashMap::new();
        for id in self.prepared.nodes.keys() {
            let Some(kit) = surface
                .node(*id)
                .and_then(|n| n.props.get("metadata"))
                .and_then(|m| m.get("pmKit"))
            else {
                continue;
            };
            if kit.get("presence") == Some(&Value::Bool(true)) {
                let shown = kit.get("closing") != Some(&Value::Bool(true));
                let amount = self.motion.presence(*id, shown, &self.table.tokens);
                presence.insert(*id, amount);
            }
        }
        self.presence = presence;
        self.motion.frame(window);
        let t = &self.table.tokens;
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .text_color(rgb(t.colors.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT)
            .child(self.element(0, window, cx, self.prepared.header))
            .child(Self::tracking(cx, self.origin.clone()))
    }
}
struct Tooltip {
    title: SharedString,
    tokens: pm_design::Tokens,
}
impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = &self.tokens;
        // Line 1 names the thing (with its shortcut); an optional line 2,
        // after a newline, says what it does (HIG help tags), in secondary.
        let (name, help) = match self.title.split_once('\n') {
            Some((name, help)) => (SharedString::from(name.to_owned()), Some(help.to_owned())),
            None => (self.title.clone(), None),
        };
        div()
            .flex()
            .flex_col()
            .px(px(t.metrics.gap))
            .py(px(t.metrics.inner_radius))
            .bg(rgb(t.colors.floating))
            // A hairline keeps it legible over a surface of its own fill
            // (the Agent popover is `floating` too).
            .border(px(t.metrics.border))
            .border_color(rgb(t.colors.hairline))
            .rounded(px(t.metrics.inner_radius))
            .text_size(px(t.typography.minimum))
            .text_color(rgb(t.colors.text))
            .max_w(px(t.metrics.value_column * 2.))
            .child(name)
            .when_some(help, |d, help| {
                d.child(div().text_color(rgb(t.colors.secondary)).child(help))
            })
    }
}
/// What to announce: statuses that appeared or changed since the last frame,
/// with non-empty text. The first frame only records them.
pub(crate) fn status_announcements(
    seen: &mut Option<HashMap<Id, String>>,
    current: impl IntoIterator<Item = (Id, String)>,
) -> Vec<String> {
    let current: HashMap<Id, String> = current.into_iter().collect();
    let messages = match seen {
        None => vec![],
        Some(previous) => {
            let mut changed: Vec<_> = current
                .iter()
                .filter(|(id, text)| !text.is_empty() && previous.get(id) != Some(text))
                .collect();
            changed.sort_by_key(|(id, _)| **id);
            changed.into_iter().map(|(_, text)| text.clone()).collect()
        }
    };
    *seen = Some(current);
    messages
}
/// A paragraph of styled runs: bold and italic as the system font draws
/// them, code on a quiet fill, links in the accent's text ink.
fn rich_text(rich: &crate::v4_compile::Rich, t: &pm_design::Tokens) -> StyledText {
    use crate::v4_compile::Rich;
    let ink = |c: u32| gpui::rgb_to_hsla(rgb(c));
    let highlights = rich.runs.iter().map(|(range, bits)| {
        let mut style = HighlightStyle::default();
        if bits & Rich::BOLD != 0 {
            style.font_weight = Some(FontWeight(t.typography.display_weight));
        }
        if bits & Rich::ITALIC != 0 {
            style.font_style = Some(FontStyle::Italic);
        }
        if bits & Rich::CODE != 0 {
            style.background_color = Some(ink(t.colors.fill));
            style.color = Some(ink(t.colors.text));
        }
        if bits & Rich::LINK != 0 {
            style.color = Some(ink(t.colors.accent_text));
        }
        (range.clone(), style)
    });
    StyledText::new(rich.text.clone()).with_highlights(highlights.collect::<Vec<_>>())
}
/// A segmented control's segments as the native control takes them (a label,
/// and a symbol where the segment is a glyph), the selected one's index (-1
/// for none) and the segments' nodes, in order.
fn segments(surface: &pm_surface::Surface, node: &pm_surface::Node) -> (Vec<Value>, i64, Vec<Id>) {
    let (mut options, mut selected, mut ids) = (vec![], -1, vec![]);
    for child in node.children.iter().filter_map(|id| surface.node(*id)) {
        if Component::of(child) != Some(Component::Segment) {
            continue;
        }
        if child.props.get("aria-checked") == Some(&Value::Bool(true)) {
            selected = ids.len() as i64;
        }
        let label = child.props.get("label").cloned().unwrap_or(Value::Null);
        options.push(
            match child.props.get("attr").and_then(|a| a.get("symbol")) {
                Some(symbol) => json!({"label": label, "symbol": symbol}),
                None => json!({"label": label}),
            },
        );
        ids.push(child.id);
    }
    (options, selected, ids)
}
/// A pop-up's rows, with the checkmarks its kit metadata carries (the
/// frozen option shape has only `v`, `label` and `disabled`).
fn native_options(node: &pm_surface::Node) -> Vec<Value> {
    let mut options = node
        .props
        .get("options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let menu = node
        .props
        .get("metadata")
        .and_then(|m| m.get("pmKit"))
        .and_then(|m| m.get("menu"))
        .and_then(Value::as_array);
    if let Some(menu) = menu {
        for option in options.iter_mut().filter(|o| o.is_object()) {
            let row = menu
                .iter()
                .find(|m| m.get("v").is_some() && m.get("v") == option.get("v"));
            // A row's checkmark and its second line (the menu item subtitle).
            for key in ["checked", "detail"] {
                if let (Some(value), Some(o)) =
                    (row.and_then(|m| m.get(key)).cloned(), option.as_object_mut())
                {
                    o.insert(key.into(), value);
                }
            }
        }
    }
    options
}
/// A native control's rows: a pop-up's options, or a slider's one range.
fn control_options(kind: crate::v4_control::Kind, node: &pm_surface::Node) -> Vec<Value> {
    if !matches!(kind, crate::v4_control::Kind::Slider) {
        return native_options(node);
    }
    let n = |k: &str| node.props.get(k).and_then(Value::as_f64);
    let log = node
        .props
        .get("metadata")
        .and_then(|m| m.get("pmKit"))
        .and_then(|m| m.get("log"))
        == Some(&Value::Bool(true));
    vec![json!({"min":n("min").unwrap_or(0.),"max":n("max").unwrap_or(1.),"step":n("step").unwrap_or(0.),"log":log})]
}
fn control_kind(panel: &str, node: &pm_surface::Node) -> Option<crate::v4_control::Kind> {
    use crate::v4_control::Kind;
    match Component::of(node) {
        Some(Component::PopUpButton) => Some(Kind::Popup),
        Some(Component::Checkbox | Component::Toggle) => Some(Kind::Checkbox),
        Some(Component::Switch) => Some(Kind::Switch),
        Some(Component::SegmentedControl) => Some(Kind::Segmented),
        Some(Component::PushButton) => Some(Kind::Push),
        // A text field its panel marks as a search (`metadata.search`).
        Some(Component::TextField)
            if node.props.get("metadata").and_then(|m| m.get("search"))
                == Some(&Value::Bool(true)) =>
        {
            Some(Kind::Search)
        }
        Some(Component::Slider) => Some(Kind::Slider),
        Some(Component::ToolbarItem)
            if panel == "export"
                && node
                    .props
                    .get("metadata")
                    .and_then(|m| m.get("export"))
                    .and_then(|m| m.get("nativeControl"))
                    .and_then(Value::as_str)
                    == Some("NSButton") =>
        {
            Some(Kind::Push)
        }
        _ => None,
    }
}
fn modified(modifiers: Modifiers) -> EventDetail {
    EventDetail {
        shift_key: Some(modifiers.shift),
        alt_key: Some(modifiers.alt),
        meta_key: Some(modifiers.platform),
        ctrl_key: Some(modifiers.control),
        ..Default::default()
    }
}
fn button_code(button: MouseButton) -> u64 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        _ => 3,
    }
}
fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

pub struct HeaderView {
    view: Entity<SurfaceView>,
    /// In a tab group: the tab names the panel, so the header's own title
    /// (its first text) is left out and only its actions stay.
    tabbed: bool,
    _observe: Subscription,
}
impl HeaderView {
    pub fn new(view: Entity<SurfaceView>, cx: &mut Context<Self>) -> Self {
        let _observe = cx.observe(&view, |_, _, cx| cx.notify());
        Self {
            view,
            tabbed: false,
            _observe,
        }
    }
    pub fn set_tabbed(&mut self, tabbed: bool, cx: &mut Context<Self>) {
        if self.tabbed != tabbed {
            self.tabbed = tabbed;
            cx.notify();
        }
    }
}
impl SurfaceView {
    /// The tooltip a node shows, if any: never one that repeats what is in
    /// view (its own words, the label of the row it sits in), and none on a
    /// control that opens a menu and shows its value (its menu says the rest,
    /// and a tooltip would stand over the open menu).
    pub(crate) fn tooltip_title(&self, id: Id) -> Option<SharedString> {
        let node = self.prepared.node(id)?;
        let shown = self.shown_text(id, 3);
        let props = self.publication.surface.node(id).map(|n| &n.props);
        let menu_value = props.is_some_and(|p| p.contains_key("aria-haspopup"))
            && (!shown.trim().is_empty()
                || props.is_some_and(|p| p.get("value").is_some_and(|v| !v.is_null())));
        node.title
            .clone()
            .filter(|title| title.trim() != shown.trim())
            .filter(|title| self.row_label(id).as_deref() != Some(title.trim()))
            .filter(|_| !menu_value)
    }
    /// The label of the row (kit `InspectorRow`) a node sits in, which the
    /// row shows beside it.
    fn row_label(&self, id: Id) -> Option<String> {
        let mut at = self.prepared.node(id)?.parent;
        while let Some(parent) = at {
            let node = self.prepared.node(parent)?;
            if node.component == Some(Component::InspectorRow) {
                return self
                    .publication
                    .surface
                    .node(parent)?
                    .props
                    .get("label")
                    .and_then(Value::as_str)
                    .map(|l| l.trim().to_owned());
            }
            at = node.parent;
        }
        None
    }
    /// The words a node shows: its own text and its descendants' (to
    /// `depth` levels), in order, without controls' values.
    pub(crate) fn shown_text(&self, id: Id, depth: usize) -> String {
        let Some(node) = self.prepared.node(id) else {
            return String::new();
        };
        let mut words: Vec<String> = node.text.iter().map(|t| t.to_string()).collect();
        if depth > 0 {
            for child in &node.children {
                let text = self.shown_text(*child, depth - 1);
                if !text.is_empty() {
                    words.push(text);
                }
            }
        }
        words.join(" ")
    }
    /// A header's title: the header itself when it is only its title
    /// (Effects), else its first child that is text (words of its own, or
    /// only words inside) and not a control.
    pub(crate) fn header_title(&self, header: Id) -> Option<Id> {
        let words = |n: &NodeView| {
            n.text.is_some()
                || (!n.children.is_empty()
                    && n.children
                        .iter()
                        .all(|c| self.prepared.node(*c).is_some_and(|k| k.is_plain_text)))
        };
        let node = self.prepared.node(header)?;
        if node.component.is_none() && words(node) {
            return Some(header);
        }
        node.children.iter().copied().find(|c| {
            self.prepared
                .node(*c)
                .is_some_and(|n| n.component.is_none() && words(n))
        })
    }
}
impl Render for HeaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabbed = self.tabbed;
        self.view.update(cx, |v, cx| match v.prepared.header {
            Some(id) => {
                let skip = if tabbed { v.header_title(id) } else { None };
                v.element(id, window, cx, skip)
            }
            None => div().into_any_element(),
        })
    }
}

fn options_len(node: &pm_surface::Node) -> usize {
    node.props
        .get("options")
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

/// A pop-up's `HostRows` (kit `rows`), from its metadata.
fn host_rows_source(node: &pm_surface::Node) -> Option<Value> {
    node.props
        .get("metadata")
        .and_then(|m| m.get("pmKit"))
        .and_then(|k| k.get("rows"))
        .filter(|r| r.is_object())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::file_drop;
    use serde_json::json;

    /// Up and Down step a number field from what is typed (else its value),
    /// ten steps with Shift, on the step's grid, within the field's limits.
    #[test]
    fn arrow_keys_step_a_number_field() {
        let facts = crate::v4_compile::NumberFacts {
            value: 0.,
            step: 0.1,
            speed: 1.,
            min: -10.,
            max: 10.,
            precision: None,
        };
        assert_eq!(super::arrow_step("0.5", facts, "up"), 0.6);
        assert_eq!(super::arrow_step("0.2", facts, "up"), 0.3);
        assert_eq!(super::arrow_step("0.5", facts, "down"), 0.4);
        assert_eq!(super::arrow_step("0.5", facts, "shiftUp"), 1.5);
        assert_eq!(super::arrow_step("", facts, "down"), -0.1);
        assert_eq!(super::arrow_step("9.95", facts, "shiftUp"), 10.);
        let whole = crate::v4_compile::NumberFacts { step: 1., ..facts };
        assert_eq!(super::arrow_step("3", whole, "down"), 2.);
        let unstepped = crate::v4_compile::NumberFacts { step: 0., ..facts };
        assert_eq!(super::arrow_step("3", unstepped, "up"), 4.);
    }

    #[test]
    fn a_file_drop_carries_paths_under_the_key_the_project_panel_reads() {
        let detail = file_drop(&["/a b/clip.mov".into(), "/c.png".into()], true);
        assert_eq!(detail.alt_key, Some(true));
        let drag = detail.drag.unwrap();
        assert_eq!(drag.types, ["public.file-url"]);
        assert_eq!(
            drag.data["public.file-url"],
            json!(["/a b/clip.mov", "/c.png"])
        );
    }
}

/// Whether the left button is down as the hardware has it (macOS); tests
/// have no hardware button and follow their events alone.
fn button_held() -> bool {
    if cfg!(test) {
        true
    } else {
        pm_shell::left_button_held()
    }
}
