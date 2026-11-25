//! GPUI host for accepted pm-surface trees. Publications compile on a worker;
//! render reads compiled facts only and never waits on JS, the kernel or disk.
//! Ordinary controls deliver events to JS; native surfaces own their own input.
use crate::v4_compile::{Kit, NodeView, Prepared, TokenTable};
use gpui::{prelude::*, *};
use pm_builtins::{Builtins, Publication};
use pm_engine_service::audio::GainPreview;
use pm_shell::text::{TextEvent, TextField, TextKind};
use pm_surface::{
    Id,
    events::{DragData, DragEffect, EventDetail},
    kit::Component,
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
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
    pub gain_preview: Rc<RefCell<Option<WeakEntity<crate::session::Controller>>>>,
    pub sample_color: Rc<RefCell<Option<crate::colour_picker::SampleSource>>>,
    /// The window's latest publication: what host-listed pop-up rows
    /// (`HostRows`) are read from when a menu opens.
    pub published: Rc<RefCell<Option<Arc<pm_kernel::Published>>>>,
}
impl Natives {
    fn send_gain_preview(&self, message: GainPreview, cx: &App) {
        if let Some(controller) = self
            .gain_preview
            .borrow()
            .as_ref()
            .and_then(WeakEntity::upgrade)
        {
            controller.read(cx).audio_gain_preview(message);
        }
    }
}
/// Scroll and column geometry the timeline surface shares with its JSX outline.
/// The outline row window stays JS; its position follows native scroll at once.
/// The rows sit in a driven scroll layer (P-SCROLL-LAYER) the surface steps
/// with its own: a wheel step inside the recorded rows replays them shifted.
#[derive(Default)]
pub struct Outline {
    scroll_y: Cell<f32>,
    pub column: Cell<f32>,
    handle: ScrollHandle,
    rows: Cell<(usize, usize)>,
    scene: RefCell<Option<Arc<pm_panel_timeline::render::Scene>>>,
    layout: RefCell<Option<TimelineOutline>>,
    waiting_for_composition: Cell<bool>,
    native_only: Cell<bool>,
    view: RefCell<Option<WeakEntity<SurfaceView>>>,
    surface: RefCell<Option<WeakEntity<crate::timeline_surface::TimelineSurface>>>,
    pub layer_dragged: Cell<bool>,
}

/// The desktop outline's mounted rows, in the order produced by its native
/// projection. The canvas can only publish geometry this outline represents.
pub(crate) struct TimelineOutline {
    document: u64,
    composition: Option<String>,
    mode: pm_panel_timeline::TimelineMode,
    total: usize,
    first: usize,
    rows: Vec<SharedString>,
}

impl TimelineOutline {
    fn from_surface(publication: &Publication, prepared: &Prepared) -> Option<Self> {
        if publication.panel != "timeline" {
            return None;
        }
        let node = |name| publication.surface.node(*prepared.semantics.get(name)?);
        let root = node("timeline")?.props.get("metadata")?;
        let document = root.get("document")?.as_str()?.parse().ok()?;
        let composition = root.get("composition")?.as_str().map(str::to_owned);
        let tree = node("timeline:tracks")?;
        let mode = match tree.props.get("label")?.as_str()? {
            "Layers" => pm_panel_timeline::TimelineMode::Layers,
            "Tracks" => pm_panel_timeline::TimelineMode::Tracks,
            _ => return None,
        };
        let total =
            usize::try_from(tree.props.get("metadata")?.get("totalRows")?.as_u64()?).ok()?;
        let (first, count) = prepared.node(tree.id)?.outline_window?;
        let rows = tree
            .children
            .iter()
            .filter_map(|id| {
                let row = publication.surface.node(*id)?;
                let id = row.props.get("metadata")?.get("nativeRow")?.as_str()?;
                Some(SharedString::from(id.to_owned()))
            })
            .collect::<Vec<_>>();
        (rows.len() == count).then_some(Self {
            document,
            composition,
            mode,
            total,
            first,
            rows,
        })
    }

    pub(crate) fn covers(
        &self,
        model: &pm_panel_timeline::TimelineModel,
        scene: &pm_panel_timeline::render::Scene,
        visible: std::ops::Range<usize>,
    ) -> bool {
        self.document == model.index.doc_id
            && self.composition == model.index.comp_id
            && self.mode == model.mode
            && self.total == scene.total()
            && visible.start >= self.first
            && visible.end <= self.first + self.rows.len()
            && visible.into_iter().all(|i| {
                scene.span(i).is_some_and(|(row, _, _)| {
                    self.rows[i - self.first].as_ref() == row.id.as_str()
                })
            })
    }

    pub(crate) fn is_for(&self, snapshot: &pm_kernel::Snapshot) -> bool {
        self.document == snapshot.doc_id && self.composition == snapshot.open_comp()
    }
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
                s.update(cx, |s, cx| {
                    s.press_property(layer.into(), path.into(), at, cx)
                });
                true
            })
    }
    fn parent_pick(&self, layer: &str, at: Point<Pixels>, cx: &mut App) {
        if let Some(surface) = self.surface.borrow().as_ref().and_then(|s| s.upgrade()) {
            surface.update(cx, |s, cx| s.press_parent(layer.into(), at, cx));
        }
    }
    fn toggle_disclosure(&self, row: &str, cx: &mut App) -> bool {
        let Ok(pm_panel_timeline::RowId::Layer(layer)) = pm_panel_timeline::v4::parse_row(row)
        else {
            return false;
        };
        if let Some(surface) = self.surface.borrow().as_ref().and_then(WeakEntity::upgrade) {
            surface.update(cx, |s, cx| s.toggle_disclosure(layer, cx));
            return true;
        }
        false
    }
    /// A layer name pressed: the surface may lift it into a reorder (the
    /// row still takes the click).
    fn press_layer(&self, layer: &str, at: Point<Pixels>, cx: &mut App) {
        self.layer_dragged.set(false);
        if let Some(surface) = self.surface.borrow().as_ref().and_then(|s| s.upgrade()) {
            surface.update(cx, |s, cx| s.press_layer_row(layer.into(), at, cx));
        }
    }
    fn move_layer(&self, event: &MouseMoveEvent, window: &mut Window, cx: &mut App) {
        if let Some(surface) = self.surface.borrow().as_ref().and_then(|s| s.upgrade()) {
            surface.update(cx, |s, cx| s.move_layer_row(event, window, cx));
        }
    }
    fn release_layer(&self, cx: &mut App) {
        if let Some(surface) = self.surface.borrow().as_ref().and_then(|s| s.upgrade()) {
            surface.update(cx, |s, cx| s.release_layer_row(cx));
        }
    }
    pub fn set_scene(&self, scene: Option<Arc<pm_panel_timeline::render::Scene>>) {
        self.waiting_for_composition.set(false);
        *self.scene.borrow_mut() = scene;
    }
    pub(crate) fn clear_composition(&self, cx: &mut App) {
        *self.scene.borrow_mut() = None;
        self.waiting_for_composition.set(true);
        if let Some(view) = self.view.borrow().as_ref().and_then(WeakEntity::upgrade) {
            cx.defer(move |cx| view.update(cx, |_, cx| cx.notify()));
        }
    }
    pub(crate) fn covers(
        &self,
        model: &pm_panel_timeline::TimelineModel,
        scene: &pm_panel_timeline::render::Scene,
        visible: std::ops::Range<usize>,
    ) -> bool {
        if !self.has_view() {
            return true;
        }
        self.layout
            .borrow()
            .as_ref()
            .is_none_or(|layout| layout.covers(model, scene, visible))
    }
    pub(crate) fn reconcile_timeline(&self, cx: &mut App) {
        if let Some(view) = self.view.borrow().as_ref().and_then(WeakEntity::upgrade) {
            cx.defer(move |cx| {
                let window = view.read(cx).window;
                let _ = cx.update_window(window, |_, window, cx| {
                    view.update(cx, |v, cx| {
                        if let Some(compiled) = v.outline_waiting.take() {
                            v.install(compiled, window, cx);
                        }
                    });
                });
            });
        }
    }
    pub fn bind(&self, view: WeakEntity<SurfaceView>) {
        *self.view.borrow_mut() = Some(view);
    }
    pub(crate) fn set_native_only(&self, native_only: bool) -> bool {
        self.native_only.replace(native_only) != native_only
    }

    pub(crate) fn has_view(&self) -> bool {
        !self.native_only.get()
            && self
                .view
                .borrow()
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .is_some()
    }
    #[cfg(test)]
    pub fn scroll_for_test(&self) -> f32 { self.scroll_y.get() }
    /// A row's top in the rows' content, when the timeline's scene has it.
    fn row_top(&self, row: &str) -> Option<f32> {
        self.scene
            .borrow()
            .as_ref()
            .and_then(|s| s.scroll_for(row))
            .map(|top| top as f32)
    }
    /// The first JS row's top in the rows' content (all rows' height).
    fn top(&self, first: &str) -> f32 {
        self.scene
            .borrow()
            .as_ref()
            .and_then(|s| s.scroll_for(first))
            .unwrap_or(0.) as f32
    }
    fn content_height(&self) -> f32 {
        self.scene
            .borrow()
            .as_ref()
            .map_or(0., |s| s.content_height()) as f32
    }
    /// A wheel step that moved only the rows: the outline's layer replays
    /// them at `scroll_y` without its view rendering again. False when the
    /// view has to render (the surface notifies it then, through
    /// [`Self::scrolled`]).
    pub fn step(
        &self,
        scroll_y: f32,
        rows: Option<(usize, usize)>,
        height: f64,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        // Keep the mounted overscan while it still covers every visible row.
        // A new suggested range alone must not redraw on each small wheel step.
        if rows.is_some_and(|rows| rows != self.rows.get()) {
            let (start, end) = self.rows.get();
            let covered = self.scene.borrow().as_ref().is_some_and(|scene| {
                let visible = scene.visible_range(scroll_y as f64, height.max(0.));
                start <= visible.start && end >= visible.end
            });
            if !covered { return false; }
        }
        let stepped =
            window.step_scroll_layer(&self.handle, point(px(0.), px(-scroll_y)), cx);
        if stepped {
            self.scroll_y.set(scroll_y);
            // Rows past JS's window are asked for as before; they arrive as
            // a publication and draw the view.
            if let (Some(rows), Some(view)) =
                (rows, self.view.borrow().as_ref().and_then(|v| v.upgrade()))
            {
                view.update(cx, |v, cx| v.outline_rows(rows, cx));
            }
        }
        stepped
    }
    pub fn scrolled(&self, scroll_y: f32, column: f32, rows: Option<(usize, usize)>, cx: &mut App) {
        let mounted = rows.unwrap_or((0, 0));
        let changed = self.rows.replace(mounted) != mounted;
        let moved = changed || self.scroll_y.get() != scroll_y || self.column.get() != column;
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
/// A virtual list's `rangechange` as last sent.
#[derive(Clone, Copy, PartialEq)]
struct RangeSent {
    range: (usize, usize),
    width: u32,
    count: usize,
    published: (usize, usize),
}
#[derive(Clone, Copy, PartialEq)]
enum RangeRequest {
    List(RangeSent),
    Outline {
        rows: (usize, usize),
        published: (usize, usize),
    },
}

/// The only pointer the host models. Multi-touch reaches panels as gestures.
const POINTER: u64 = 1;

#[path = "v4_grade.rs"]
mod grade;

#[path = "v4_cull.rs"]
mod cull;

/// A number field's value after an arrow key: the typed text (else the
/// field's value) one step up or down, ten with Shift, within its limits.
pub(crate) fn arrow_step(typed: &str, facts: crate::v4_compile::NumberFacts, key: &str) -> f64 {
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

/// How long a panel's demand must stay put before the shell reads it again:
/// a scroll layer's settle (rows measured as a scroll brings them near).
const DEMAND_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// A press on a number field: a click edits, a drag scrubs (language.md §9).
/// A press on a grading control (`grade`) drags it through the same gesture.
struct ScrubDrag {
    id: Id,
    start: Point<Pixels>,
    number: crate::v4_compile::NumberFacts,
    motion: pm_surface::kit::NumericMotion,
    pub(super) infinite: crate::infinite_drag::InfiniteDrag,
    binding: Option<SharedString>,
    active: bool,
    grade: Option<grade::GradeDrag>,
    // Local input previews, then one change on release; opt-in controls only.
    local_commit: Option<(Value, Value)>,
}
struct GainScrub {
    gesture: u64,
    binding: SharedString,
    value: f64,
    request: pm_kernel::Request,
}
/// An undo step's name from a control's label: the label without the
/// accessibility suffix a colour control carries.
fn undo_name_of(label: &str) -> Option<String> {
    let name = label
        .strip_suffix(" colour")
        .or_else(|| label.strip_suffix(" color"))
        .unwrap_or(label)
        .trim();
    (!name.is_empty()).then(|| name.to_owned())
}
#[cfg(test)]
mod undo_name_tests {
    #[test]
    fn a_colour_drag_undoes_by_its_controls_name() {
        assert_eq!(super::undo_name_of("Lift colour").as_deref(), Some("Lift"));
        assert_eq!(super::undo_name_of("Curve").as_deref(), Some("Curve"));
        assert_eq!(super::undo_name_of("Temperature").as_deref(), Some("Temperature"));
        assert_eq!(super::undo_name_of(" "), None);
    }
}
enum ScrubOp {
    /// The undo step's name: the control's (Lift, Curve, Temperature).
    Begin(Option<String>),
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
    /// Native layer reorder owns this captured pointer, including its moves.
    layer_drag: bool,
}
/// A scroller anchored at its end (`metadata.scrollAnchor: "end"`): the
/// panel hears when the reader scrolls away from the end and back (a
/// `scroll` event, `{away}`) so it can offer the way back, and a new
/// `metadata.scrollToken` takes it there.
#[derive(Default)]
struct Follow {
    token: Value,
    anchor_start: bool,
    compact: Option<bool>,
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
    outline: Option<TimelineOutline>,
}
pub struct SurfaceView {
    panel_build: Option<crate::generated_panel::panel_build::State>,
    /// The `metadata.hoverGroup` nodes (rows) the pointer is over, as this
    /// render found them: each group painted in the last frame whose hitbox
    /// the pointer is in now. The host draws their hover (`hoverStyle`,
    /// `hoverOnly`, `hoverHide`), so a row's hover is no publication of the
    /// panel. Derived every render, never kept across one: a group that left
    /// the frame (scrolled out of the rendered rows, deleted, another
    /// generation) is hovered by nothing.
    hovered_groups: BTreeSet<Id>,
    /// The hover groups the last frame painted, with their hitboxes; filled
    /// while painting, taken by the next render.
    painted_groups: Rc<RefCell<HashMap<Id, gpui::HitboxId>>>,
    build_painted: Cell<u64>,
    pub runtime: Arc<Builtins>,
    pub publication: Publication,
    table: Arc<TokenTable>,
    pub(crate) prepared: Rc<Prepared>,
    window: AnyWindowHandle,
    appkit: bool,
    quote_selection: Rc<RefCell<crate::v4_quote::Selection>>,
    natives: Natives,
    fields: BTreeMap<Id, Entity<TextField>>,
    colour: Option<(Id, Entity<crate::colour_picker::Picker>, Subscription)>,
    controls: BTreeMap<Id, Entity<crate::v4_control::NativeControl>>,
    /// Rows of a long scroller left out of a frame (`v4_cull`).
    cull: cull::Cull,
    /// Pop-ups whose rows the host lists, by the source they were given.
    row_sources: HashMap<Id, (EntityId, Value)>,
    /// Disclosure buttons pressed whose group the script has not yet shown
    /// opened or closed: the chevron and the group's content follow the
    /// press in the next frame, the panel's own work after it.
    pub(crate) disclosures: HashMap<Id, bool>,
    /// A text field to focus once it is mounted (the Agent composer when its
    /// popover opens) and what was typed before it was.
    ready_text: Option<(String, String, std::time::Instant)>,
    /// The control the left button is down on: it shows pressed (a deeper
    /// wash than hover) until the button comes up, as a Mac button does.
    pressed: Option<Id>,
    scrolls: BTreeMap<Id, ScrollHandle>,
    leases: BTreeMap<Id, Option<SharedString>>,
    option_sets: BTreeMap<Id, Value>,
    focus: BTreeMap<Id, FocusHandle>,
    keyboard_focused_user_message: Option<Id>,
    /// Hover washes (pm-design springs), stepped once per render.
    motion: pm_shell::motion::Motion<Id>,
    /// Floating surfaces marked `pmKit.presence` (the Light panel): how far
    /// in they are this frame, 0–1. Set before the motion frame steps.
    presence: HashMap<Id, f32>,
    subscriptions: BTreeMap<Id, Vec<Subscription>>,
    _escape_intercept: Subscription,
    focus_events: std::collections::BTreeSet<Id>,
    origin: Rc<Cell<Point<Pixels>>>,
    /// Counts demand changes, so only the last one's settle draws again.
    demand_changes: u64,
    anchors: Rc<RefCell<HashMap<Id, Bounds<Pixels>>>>,
    truncated: Rc<RefCell<HashMap<Id, bool>>>,
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
    gain_scrub: Option<GainScrub>,
    /// A horizontal wheel scrub: its node, its scrub, whether it writes a
    /// binding, and the number it started from (a cancel shows it again).
    wheel_scrub: Option<(Id, pm_surface::kit::HorizontalScrub, bool, f64)>,
    wheel_tick: u64,
    wheel_lifted: bool,
    scrub_ops: VecDeque<ScrubOp>,
    /// The panel item being dragged (its `dragstart` ran): `dragend` goes to
    /// it once the drag is over, wherever it was dropped.
    dragging: Option<Id>,
    /// Each virtual list's width last told to its panel.
    list_widths: HashMap<Id, (f32, f32)>,
    /// Each virtual list's last `rangechange` (range, width, its geometry):
    /// a wheel step within the same rows sends nothing, so the panel doesn't
    /// commit and draw again (PM-311).
    sent_ranges: HashMap<Id, RangeRequest>,
    pending_ranges: HashMap<Id, (RangeRequest, Task<()>)>,
    pending_samples: HashMap<Id, Task<()>>,
    scrub_gesture: Option<u64>,
    scrub_busy: bool,
    scrub_shown: Option<(Id, f64)>,
    /// Keep the final readout through compilations already queued at commit.
    scrub_settled: Option<(Id, f64, u64)>,
    grade_drawn: grade::Drawn,
    /// Each wheel's puck on its way to where its values put it.
    grade_motion: grade::Motion,
    /// Each easing curve's preview run and the curve its drag left, until
    /// the panel publishes it.
    ease: grade::EaseState,
    pointer_move: Option<Task<()>>,
    /// Last label of each `role="status"` node; `None` until the first frame,
    /// so a panel's opening state is not read out.
    statuses: Option<HashMap<Id, String>>,
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
    /// The newest publication this view was handed (shown, compiling,
    /// parked or waiting for the timeline's rows): what a retheme compiles,
    /// so it never puts an older tree back over one still on its way.
    received: Option<Publication>,
    outline_waiting: Option<Compiled>,
    /// A text layer whose Properties text to focus once its row shows
    /// (`inspector.focusText`), until the moment passes.
    focus_text: Option<(String, std::time::Instant)>,
    /// The field the panel last asked to take the keyboard (`focusRequest`).
    focus_requested: Option<String>,
    /// A semantic id to scroll into view once a publication has it.
    reveal: Option<(String, std::time::Instant)>,
    /// Told when a new tree is installed: the timeline's native timecode
    /// layer steps aside while its time field is open, and back after.
    pub(crate) installed: Option<async_channel::Sender<()>>,
    /// The panel sits in a body that scrolls (every side panel): its root
    /// is at least the body (`fit`) and grows with content taller than it,
    /// whose height (`natural`) sizes the body's content, so the body has
    /// something to scroll. The viewer and the timeline fit their slot.
    pub(crate) grows: bool,
    fit: Cell<f32>,
    natural: Rc<Cell<f32>>,
    column_natural: Rc<Cell<f32>>,
    agent_composer_height: Rc<Cell<f32>>,
    #[cfg(test)]
    pub renders: usize,
    #[cfg(test)]
    pub log: Vec<(Id, String, EventDetail)>,
    #[cfg(test)]
    number_edit_request: Option<Id>,
    pub error: Option<String>,
}
/// The height of a scroller's content: where its last child ends below its
/// top, in layout (which its offset does not move, nor its own height).
fn content_height(scroll: &ScrollHandle) -> f32 {
    let top = scroll.bounds().top();
    let mut bottom = top;
    for ix in 0.. {
        let Some(child) = scroll.bounds_for_item(ix) else {
            break;
        };
        bottom = bottom.max(child.bottom());
    }
    f32::from(bottom - top)
}

/// Fades over a scroller's viewport where content runs past its top or
/// bottom edge, into `ground` (the colour under it), a row tall: a region
/// that draws no scroller still shows there is more. Drawn over the content
/// in the viewport's own place, whatever the scroll.
fn scroll_edges(scroll: ScrollHandle, ground: u32, t: &pm_design::Tokens) -> AnyElement {
    let depth = px(t.metrics.form_row);
    canvas(
        |_, _, _| {},
        move |_, _, window, _| {
            let view = scroll.bounds();
            let (offset, max) = (-scroll.offset().y, scroll.max_offset().y);
            if max <= px(0.) || view.size.height <= depth * 2. {
                return;
            }
            let fade = |top: bool| {
                let (from, to) = if top { (1., 0.) } else { (0., 1.) };
                gpui::linear_gradient(
                    180.,
                    gpui::linear_color_stop(pm_shell::kit::ink(ground, from), 0.),
                    gpui::linear_color_stop(pm_shell::kit::ink(ground, to), 1.),
                )
            };
            if offset > px(1.) {
                let band = Bounds::new(view.origin, size(view.size.width, depth));
                window.paint_quad(gpui::fill(band, fade(true)));
            }
            if offset < max - px(1.) {
                let band = Bounds::new(
                    point(view.origin.x, view.bottom() - depth),
                    size(view.size.width, depth),
                );
                window.paint_quad(gpui::fill(band, fade(false)));
            }
        },
    )
    .absolute()
    .size_0()
    .into_any_element()
}
impl SurfaceView {
    /// Seed a synthetic hover for headless captures without OS input.
    #[cfg(test)]
    pub(crate) fn capture_composition_tab_hover(&mut self, hovered: bool) {
        if !hovered { return; }
        let tabs = || self.publication.surface.nodes().values()
            .filter(|n| n.props.get("role").and_then(Value::as_str) == Some("tab"));
        let id = tabs().find(|n| n.props.get("aria-selected") == Some(&Value::Bool(false)))
            .or_else(|| tabs().next()).map(|n| n.id);
        if let Some(id) = id {
            self.motion.hovers.hover(id, true, &self.table.tokens);
        }
    }

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
        if publication.panel == "timeline" {
            *natives.outline.layout.borrow_mut() =
                TimelineOutline::from_surface(&publication, &prepared);
        }
        cx.on_release(|view, cx| {
            if let Some(scrub) = view.gain_scrub.take() {
                let natives = view.natives.clone();
                cx.defer(move |cx| {
                    natives.send_gain_preview(
                        GainPreview::Finish {
                            gesture: scrub.gesture,
                            revision: None,
                        },
                        cx,
                    )
                });
            }
        })
        .detach();
        let mut this = Self {
            panel_build: None,
            hovered_groups: BTreeSet::new(),
            painted_groups: Rc::default(),
            build_painted: Cell::new(u64::MAX),
            runtime,
            publication,
            table,
            prepared,
            window: (*window).window_handle(),
            appkit: HasWindowHandle::window_handle(window).is_ok(),
            natives,
            quote_selection: Default::default(),
            fields: BTreeMap::new(),
            colour: None,
            controls: BTreeMap::new(),
            cull: Default::default(),
            row_sources: HashMap::new(),
            disclosures: HashMap::new(),
            scrolls: BTreeMap::new(),
            leases: BTreeMap::new(),
            option_sets: BTreeMap::new(),
            focus: BTreeMap::new(),
            keyboard_focused_user_message: None,
            motion: Default::default(),
            pressed: None,
            ready_text: None,
            presence: HashMap::new(),
            subscriptions: BTreeMap::new(),
            _escape_intercept: {
                let cancelled = cx.entity().downgrade();
                cx.intercept_keystrokes(move |event, window, cx| {
                    if event.keystroke.key == "escape" {
                        let _ = cancelled.update(cx, |v, cx| {
                            if v.scrub.is_some() && v.cancel_gesture(cx) {
                                cx.stop_propagation();
                            } else if v.dragging.is_some() {
                                cx.stop_active_drag(window);
                                v.end_drag(cx);
                                cx.stop_propagation();
                            }
                        });
                    }
                })
            },
            focus_events: Default::default(),
            origin: Rc::new(Cell::new(Point::default())),
            demand_changes: 0,
            anchors: Default::default(),
            truncated: Default::default(),
            sheet_fit: Default::default(),
            follow: Default::default(),
            probing: false,
            targets: Default::default(),
            capture: None,
            scrub: None,
            gain_scrub: None,
            wheel_scrub: None,
            wheel_tick: 0,
            wheel_lifted: false,
            scrub_ops: VecDeque::new(),
            dragging: None,
            list_widths: HashMap::new(),
            sent_ranges: HashMap::new(),
            pending_ranges: HashMap::new(),
            pending_samples: HashMap::new(),
            scrub_gesture: None,
            scrub_busy: false,
            scrub_shown: None,
            scrub_settled: None,
            grade_drawn: Default::default(),
            grade_motion: Default::default(),
            ease: Default::default(),
            pointer_move: None,
            statuses: None,
            latest: 0,
            shown: 0,
            handling: 0,
            wake_idle: false,
            parked: None,
            received: None,
            outline_waiting: None,
            focus_text: None,
            focus_requested: None,
            reveal: None,
            installed: None,
            grows: false,
            fit: Cell::new(0.),
            natural: Rc::new(Cell::new(0.)),
            column_natural: Rc::new(Cell::new(0.)),
            agent_composer_height: Rc::new(Cell::new(0.)),
            #[cfg(test)]
            renders: 0,
            #[cfg(test)]
            log: vec![],
            #[cfg(test)]
            number_edit_request: None,
            error: None,
        };
        this.reconcile(window, cx);
        this
    }
    fn build_rows(&self) -> Vec<Id> {
        fn walk(view: &SurfaceView, id: Id, rows: &mut Vec<Id>) {
            let Some(node) = view.prepared.node(id) else {
                return;
            };
            if node.component == Some(Component::InspectorRow) {
                rows.push(id);
            }
            for child in &node.children {
                walk(view, *child, rows);
            }
        }
        let mut rows = Vec::new();
        walk(self, 0, &mut rows);
        rows
    }
    pub(crate) fn panel_build_rendered(&self, cx: &App) -> bool {
        self.shown == self.latest
            && self.build_painted.get() == self.shown
            && self
                .controls
                .iter()
                .all(|(id, c)| self.left_out(*id) || !c.read(cx).snapshot_pending())
            && self.fields.values().all(|f| !f.read(cx).snapshot_pending())
    }
    pub(crate) fn set_panel_build(
        &mut self,
        state: Option<crate::generated_panel::panel_build::State>,
        cx: &mut Context<Self>,
    ) {
        let state = state.filter(|s| s.resolve(s.now()) < 1.);
        // No build now and none before: every control and field already
        // shows none (they start without one, and the call that ended the
        // last build cleared them). The panel asks on every frame it draws.
        if state.is_none() && self.panel_build.is_none() {
            return;
        }
        if let Some(ready) = state
            .as_ref()
            .filter(|s| s.stage == crate::generated_panel::panel_build::Stage::Ready)
        {
            if self
                .panel_build
                .as_ref()
                .is_none_or(|old| old.changed != ready.changed || old.stage != ready.stage)
            {
                let changed = ready.changed;
                let deadline =
                    changed + std::time::Duration::from_secs_f32(pm_design::dots::RESOLVE_SECONDS);
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(deadline.saturating_duration_since(std::time::Instant::now()))
                        .await;
                    let _ = this.update(cx, |view, cx| {
                        if view.panel_build.as_ref().is_some_and(|s| {
                            s.changed == changed
                                && s.stage == crate::generated_panel::panel_build::Stage::Ready
                        }) {
                            view.set_panel_build(None, cx);
                        }
                    });
                })
                .detach();
            }
        }
        let rows = self.build_rows();
        let mut controls = self.controls.iter().collect::<Vec<_>>();
        controls.sort_by_key(|(id, _)| **id);
        for (fallback, (id, control)) in controls.into_iter().enumerate() {
            let mut parent = Some(*id);
            let mut row = None;
            while let Some(id) = parent {
                if let Some(index) = rows.iter().position(|r| *r == id) {
                    row = Some(index);
                    break;
                }
                parent = self.prepared.node(id).and_then(|n| n.parent);
            }
            let index = row.unwrap_or(fallback);
            let mode = state.as_ref().map(|s| {
                let index = if s.explicit { index } else { 0 };
                let revision = s
                    .committed
                    .get(&index)
                    .map(|at| at.saturating_duration_since(s.started).as_nanos() as u64);
                let ready = (s.stage == crate::generated_panel::panel_build::Stage::Ready)
                    .then(|| s.resolve(s.now()));
                let filter = if row.is_none() {
                    crate::generated_panel::dot_filter(Some(s), Some(index), &self.table.tokens)
                        .into_iter()
                        .next()
                } else {
                    None
                };
                pm_shell::native_control::BuildSnapshot {
                    revision,
                    ready,
                    filter,
                }
            });
            control.update(cx, |control, cx| {
                if control.set_build_snapshot(mode) {
                    cx.notify();
                }
            });
        }
        for (fallback, (id, field)) in self.fields.iter().enumerate() {
            let mut parent = Some(*id);
            let mut row = None;
            while let Some(id) = parent {
                if let Some(index) = rows.iter().position(|r| *r == id) {
                    row = Some(index);
                    break;
                }
                parent = self.prepared.node(id).and_then(|n| n.parent);
            }
            let index = row.unwrap_or(fallback);
            let mode = state.as_ref().map(|s| {
                let index = if s.explicit { index } else { 0 };
                let revision = s
                    .committed
                    .get(&index)
                    .map(|at| at.saturating_duration_since(s.started).as_nanos() as u64);
                let ready = (s.stage == crate::generated_panel::panel_build::Stage::Ready)
                    .then(|| s.resolve(s.now()));
                let filter = if row.is_none() {
                    crate::generated_panel::dot_filter(Some(s), Some(index), &self.table.tokens)
                        .into_iter()
                        .next()
                } else {
                    None
                };
                pm_shell::native_control::BuildSnapshot {
                    revision,
                    ready,
                    filter,
                }
            });
            field.update(cx, |field, cx| {
                if field.set_build_snapshot(mode) {
                    cx.notify();
                }
            });
        }
        let active = self.panel_build.is_some() || state.is_some();
        self.panel_build = state;
        if active {
            cx.notify();
        }
    }
    /// A reopened panel window keeps its view; later publications install
    /// there, and so does one that finished while no window was open.
    pub fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        if handle != self.window {
            self.keyboard_focused_user_message = None;
            self.painted_groups.borrow_mut().clear();
        }
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
            self.focus_events.clear();
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
    /// The already drawn window scene supplies library covers without mounting
    /// another interactive view or requesting an editor redraw.
    pub(super) fn library_window(&self) -> AnyWindowHandle {
        self.window
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
        self.received = Some(publication.clone());
        self.compile(publication, self.table.clone(), cx);
    }
    pub fn retheme(&mut self, tokens: pm_design::Tokens, cx: &mut Context<Self>) {
        let table = Arc::new(TokenTable::new(tokens));
        // The newest tree handed over, wherever it is: its compile is
        // superseded by this one (only the newest ticket installs), and a
        // later commit equal to it is not published again (PM-698 review).
        let publication = self
            .received
            .clone()
            .unwrap_or_else(|| self.publication.clone());
        self.compile(publication, table, cx);
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
            let source = publication.clone();
            let worker = table.clone();
            let (prepared, outline) = cx
                .background_executor()
                .spawn(async move {
                    let started = std::time::Instant::now();
                    let prepared = Prepared::new(source.surface.clone(), &worker);
                    let outline = TimelineOutline::from_surface(&source, &prepared);
                    if crate::diag::enabled() {
                        crate::diag::line(serde_json::json!({"ui": "prepared",
                            "ms": started.elapsed().as_secs_f64() * 1000.}));
                    }
                    (prepared, outline)
                })
                .await;
            let mut compiled = Some(Compiled {
                ticket,
                outline,
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
        let timeline = self
            .natives
            .outline
            .surface
            .borrow()
            .as_ref()
            .and_then(WeakEntity::upgrade);
        if let (Some(layout), Some(timeline)) = (&compiled.outline, &timeline)
            && !timeline.read(cx).can_show_outline(layout)
        {
            self.outline_waiting = Some(compiled);
            if let Some(rows) = timeline.read(cx).pending_outline_range() {
                self.outline_rows(rows, cx);
            }
            return;
        }
        self.outline_waiting = None;
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
            // Another realm's tree: its node ids are not the last frame's.
            self.painted_groups.borrow_mut().clear();
            self.wheel_finish(false, cx);
            self.restart(cx);
        }
        let quote_owner = self.quote_selection.borrow().owner;
        if let Some(owner) = quote_owner {
            let source = |surface: &pm_surface::Surface| {
                surface
                    .node(owner)
                    .and_then(|n| n.props.get("metadata"))
                    .and_then(|m| m.get("quoteSource"))
                    .cloned()
            };
            if source(&self.publication.surface) != source(&compiled.publication.surface) {
                // The same row id in another thread is a different source.
                self.quote_selection.borrow_mut().owner = None;
            }
        }
        self.table = compiled.table;
        if compiled.publication.panel == "timeline" {
            *self.natives.outline.layout.borrow_mut() = compiled.outline;
            if let Some(timeline) = timeline {
                cx.defer(move |cx| timeline.update(cx, |t, cx| t.show_pending_scene(cx)));
            }
        }
        self.publication = compiled.publication;
        self.prepared = Rc::new(compiled.prepared);
        if self.scrub_settled.is_some_and(|(id, value, ticket)| {
            compiled.ticket > ticket || self.prepared.node(id).and_then(|n| n.number)
                .is_none_or(|n| n.text(n.value) == n.text(value))
        }) {
            self.scrub_settled = None;
        }
        self.quote_selection
            .borrow_mut()
            .blocks
            .retain(|id, _| self.prepared.node(*id).is_some());
        // Where rows were is the old tree's (a search moved them up); their
        // heights stay good guesses. Rows left out by old tops never showed.
        self.cull.forget_tops();
        self.settle_disclosures();
        {
            let _span = crate::diag::span("panel.reconcile");
            self.reconcile(window, cx);
        }
        self.try_focus_text(cx);
        self.follow_focus_request(cx);
        self.take_ready(cx);
        if self.publication.panel == "timeline" {
            self.sync_outline_window(cx);
        }
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
    fn restart(&mut self, cx:&mut Context<Self>) {
        if let Some((_,picker,_))=self.colour.take() {picker.update(cx,|p,_|p.cancel_pending());}
        self.quote_selection = Default::default();
        self.disclosures.clear();
        self.sent_ranges.clear();
        self.list_widths.clear();
        self.pending_ranges.clear();
        self.pending_samples.clear();
        self.controls.clear();
        self.fields.clear();
        self.scrolls.clear();
        self.follow.get_mut().clear();
        self.agent_composer_height.set(0.);
        self.leases.clear();
        self.option_sets.clear();
        self.focus.clear();
        self.keyboard_focused_user_message = None;
        self.subscriptions.clear();
        self.focus_events.clear();
        self.anchors.borrow_mut().clear();
        self.truncated.borrow_mut().clear();
        self.capture = None;
        self.scrub = None;
        self.scrub_shown = None;
        self.scrub_settled = None;
        self.pointer_move = None;
        self.error = None;
    }
    fn reconcile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.announce_statuses(window);
        let prepared = self.prepared.clone();
        let live = |id: &Id| prepared.nodes.contains_key(id);
        if self.colour.as_ref().is_some_and(|(id,_,_)| !live(id) || self.leases.get(id) != prepared.node(*id).map(|n| &n.binding)) {
            if let Some((_,picker,_))=self.colour.take() {picker.update(cx,|p,_|p.cancel_pending());}
            if self.scrub_busy || self.scrub_gesture.is_some() || !self.scrub_ops.is_empty() {self.scrub_ops.push_back(ScrubOp::Finish(false));self.scrub_pump(cx);}
        }

        self.controls.retain(|id, _| live(id));
        if self.pressed.is_some_and(|id| !live(&id)) {
            self.pressed = None;
        }
        self.cull.retain(|id| live(id));
        self.truncated.borrow_mut().retain(|id, _| live(id));
        self.row_sources.retain(|id, _| live(id));
        self.scrolls.retain(|id, _| prepared.node(*id).is_some_and(|n| n.scroll_y || n.scroll_x));
        self.sent_ranges.retain(|id, _| live(id));
        self.list_widths.retain(|id, _| live(id));
        self.pending_ranges.retain(|id, _| live(id));
        self.pending_samples.retain(|id, _| live(id));
        self.subscriptions.retain(|id, _| live(id));
        self.fields.retain(|id, _| {
            if !live(id) {
                pm_mcp::coedit::touch_field(
                    self.runtime.project_id(),
                    &format!("{}/{}", self.publication.panel, id),
                    false,
                );
            }
            live(id)
        });
        self.focus.retain(|id, _| live(id));
        self.keyboard_focused_user_message =
            self.keyboard_focused_user_message.filter(|id| live(id));
        self.focus_events.retain(|id| live(id));
        self.leases.retain(|id, _| live(id));
        self.option_sets.retain(|id, _| live(id));
        // A header that auto-scrolled off screen still owns its native drag.
        if self
            .capture
            .is_some_and(|c| !c.layer_drag && !live(&c.target))
        {
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
                if !self.controls.contains_key(&id) && self.cull_large() && self.in_scroller(id) {
                    // Made when its row is first built (`v4_cull`).
                    self.cull.deferred.insert(id, kind);
                } else {
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
                }
                // Segmented controls own their Tab stop in the kit.
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
        // Glyph actions deliver focus to their row as well as pointer hover.
        // Text fields and AppKit form controls keep their existing lifecycle.
        for (&id, handle) in &self.focus {
            if self.focus_events.contains(&id) {
                continue;
            }
            let user_message = self.publication.panel == "agent"
                && surface.node(id)
                    .and_then(|node| node.props.get("metadata"))
                    .is_some_and(|metadata| {
                        metadata.get("quoteSource").is_some()
                            && metadata.get("userMessage").and_then(Value::as_bool) == Some(true)
                    });
            if user_message {
                let focused = cx.on_focus(handle, window, move |v, window, cx| {
                    v.keyboard_focused_user_message =
                        window.last_input_was_keyboard().then_some(id);
                    // GPUI delivers focus callbacks during drawing. Invalidate
                    // the view after the current frame so the ring is painted.
                    cx.defer_in(window, |_, _, cx| cx.notify());
                });
                let blurred = cx.on_blur(handle, window, move |v, _, _| {
                    if v.keyboard_focused_user_message == Some(id) {
                        v.keyboard_focused_user_message = None;
                    }
                });
                self.subscriptions.entry(id).or_default().extend([focused, blurred]);
                self.focus_events.insert(id);
                continue;
            }
            if !prepared.node(id).is_some_and(|node| node.component == Some(Component::ToolbarItem)) {
                continue;
            }
            let mut at = Some(id);
            let mut interested = false;
            while let Some(target) = at {
                let Some(node) = surface.node(target) else { break; };
                interested |= node.listeners.contains_key("focus") || node.listeners.contains_key("blur");
                at = node.parent;
            }
            if !interested { continue; }
            let focused = cx.on_focus(handle, window, move |v, _, cx| {
                v.pointer(id, "focus", EventDetail::default(), cx);
            });
            let blurred = cx.on_blur(handle, window, move |v, window, cx| {
                // Carry the next control's semantic name so a row retains its
                // reveal while Tab moves between actions in that same row.
                let next = v.publication.surface.node(id).filter(|node|node.input.is_none())
                    .and_then(|_|v.focused_node(window)).and_then(|next|v.publication.surface.node(next))
                    .and_then(|node|node.props.get("id")).cloned();
                v.pointer(id, "blur", EventDetail { value: next, ..Default::default() }, cx);
            });
            self.subscriptions.entry(id).or_default().extend([focused, blurred]);
            self.focus_events.insert(id);
        }
        if let Some(build) = self.panel_build.clone() {
            self.set_panel_build(Some(build), cx);
        }
        if crate::diag::enabled() {
            crate::diag::line(
                serde_json::json!({"ui": "reconcile", "nodes": prepared.nodes.len(),
                "controls": mounts.0, "controls_ms": mounts.1.as_secs_f64() * 1000.,
                "fields": mounts.2, "fields_ms": mounts.3.as_secs_f64() * 1000.}),
            );
        }
    }
    /// A drag's undo step, named for its control: its label without the
    /// accessibility suffix ("Lift colour" undoes as "Lift").
    pub(super) fn undo_name(&self, id: Id) -> Option<String> {
        undo_name_of(self.publication.surface.node(id)?.props.get("label")?.as_str()?)
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
    /// A control whose row is built for the first time (`v4_cull`).
    fn mount_deferred(
        &mut self,
        id: Id,
        kind: crate::v4_control::Kind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (prepared, surface) = (self.prepared.clone(), self.publication.surface.clone());
        let (Some(view), Some(node)) = (prepared.node(id), surface.node(id)) else {
            return;
        };
        self.mount_control(kind, view, node, view.binding.clone(), window, cx);
        self.host_rows(id, node, cx);
        if let Some(build) = self.panel_build.clone() {
            self.set_panel_build(Some(build), cx);
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
        // A segmented control in the panel's header is its mode switch, at
        // the header's size; elsewhere the system's regular size.
        let header_switch = matches!(kind, crate::v4_control::Kind::Segmented)
            && (self.in_header(id) || crate::v4_compile::variant(node) == Some("glass"));
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
        let authored_style = node.props.get("style").unwrap_or(&Value::Null);
        let popup_width = self.table.number(&authored_style["width"]);
        let popup_minimum = self.table.number(&authored_style["minWidth"]);
        if let Some(control) = self.controls.get(&id) {
            control.update(cx, |v, _| {
                v.display(display.as_deref());
                v.set_tab_index(view.tab_index.unwrap_or(0));
                if matches!(kind, crate::v4_control::Kind::PopupTrigger) {
                    v.menu_open(node.props.get("open") == Some(&Value::Bool(true)));
                    v.popup_width(popup_width, popup_minimum);
                }
            });
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
            let mut control = crate::v4_control::NativeControl::new(
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
                    inline: matches!(
                        crate::v4_compile::variant(node),
                        Some("inline" | "quiet" | "pulldown")
                    ) && matches!(kind, crate::v4_control::Kind::Popup),
                    well: view.well,
                    quiet: crate::v4_compile::variant(node) == Some("quiet"),
                    pull: crate::v4_compile::variant(node) == Some("pulldown"),
                    tint: None,
                    symbol: None,
                    // Every segmented control is the kit's one: the system's
                    // Liquid Glass capsule (PM-409); large as a header's mode
                    // switch, small in a row's value well.
                    glass: header_switch,
                },
                &tokens,
                window,
                cx,
            );
            control.set_tab_index(view.tab_index.unwrap_or(0));
            control
        });
        let generation = self.publication.generation;
        self.subscriptions.entry(id).or_default().push(cx.subscribe(
            &control,
            move |v, _, value, cx| {
                if v.publication.generation != generation || v.leases.get(&id) != Some(&lease) {
                    return;
                }
                if matches!(
                    kind,
                    crate::v4_control::Kind::Push | crate::v4_control::Kind::PopupTrigger
                ) {
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
        control.update(cx, |v, _| {
            v.display(display.as_deref());
            if matches!(kind, crate::v4_control::Kind::PopupTrigger) {
                v.menu_open(node.props.get("open") == Some(&Value::Bool(true)));
                v.popup_width(popup_width, popup_minimum);
            }
        });
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
            (Some(number), _) => {
                number.text(self.scrub_readout(id).unwrap_or(number.value))
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
        let composer_epoch = node
            .props
            .get("metadata")
            .and_then(|m| m.get("composerEpoch"))
            .and_then(Value::as_u64);
        let composer_sending = node
            .props
            .get("metadata")
            .and_then(|m| m.get("composerSending"))
            .and_then(Value::as_str);
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
                match (composer_epoch, host) {
                    (Some(epoch), _) => f.set_composer_value(&value, epoch, composer_sending, cx),
                    (None, Some(revision)) => f.set_value_from_host(&value, revision, cx),
                    (None, None) => f.set_value(&value, cx),
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
        // A form's field (Settings, `metadata.form`): AppKit's bordered field
        // shown at all times; a number one may carry a stepper.
        let meta = node.props.get("metadata");
        let form = meta.and_then(|m| m.get("form")) == Some(&Value::Bool(true));
        let stepper = meta.and_then(|m| m.get("stepper")) == Some(&Value::Bool(true));
        let kind = if form && number {
            TextKind::FormNumber
        } else if form && node.r#type != "textarea" {
            TextKind::Form
        } else if number {
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
            field.set_inline_name(node.props.get("metadata").and_then(|m| m.get("inlineRename")) == Some(&Value::Bool(true)));
            if view.code {
                field.set_code(pm_design::MONO_FONT);
            }
            field.set_ink(Self::ink(view));
            field.set_placeholder(&placeholder);
            field.set_unit(unit);
            field.set_stepper(stepper);
            if let Some(epoch) = composer_epoch {
                field.set_composer_value(&value, epoch, composer_sending, cx);
            }
            field
        });
        if kind == TextKind::Composer
            && node.props.get("id").and_then(Value::as_str) == Some("agent-composer-text")
        {
            let project = meta
                .and_then(|m| m.get("attachmentProject"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let thread = meta
                .and_then(|m| m.get("attachmentThread"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let (tx, rx) = async_channel::unbounded();
            field.update(cx, |f, _| {
                f.set_paste_handler(std::rc::Rc::new(move || {
                    let Some(media) = crate::platform::agent_pasteboard() else {
                        return false;
                    };
                    tx.try_send(media).is_ok()
                }))
            });
            let runtime = self.runtime.clone();
            cx.spawn(async move |this, cx| {
                while let Ok(media) = rx.recv().await {
                    let runtime = runtime.clone();
                    let (project, thread) = (project.clone(), thread.clone());
                    let outcome = cx
                        .background_executor()
                        .spawn(async move {
                            runtime.agent_attach_for(
                                &project,
                                &thread,
                                crate::media::agent_pasted_records(media.into_media()?)?,
                            )
                        })
                        .await;
                    if let Err(error) = outcome {
                        let _ = this.update(cx, |v, cx| {
                            v.error = Some(error);
                            cx.notify();
                        });
                    }
                }
            })
            .detach();
        }
        self.subscriptions
            .entry(id)
            .or_default()
            .push(
                cx.subscribe_in(&field, window, move |this, _, event, window, cx| {
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
                            if lease.is_some() {
                                pm_mcp::coedit::touch_field(
                                    this.runtime.project_id(),
                                    &format!("{}/{}", this.publication.panel, id),
                                    true,
                                );
                            }
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
                            pm_mcp::coedit::touch_field(
                                this.runtime.project_id(),
                                &format!("{}/{}", this.publication.panel, id),
                                false,
                            );
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
                        // Carry the editor text with Return. Publishing a separate
                        // input first could replay a draft already accepted by Send.
                        TextEvent::Submitted(value) => {
                            let (listens_input, listens_keydown) = {
                                let listens = |event: &str| {
                                    this.publication
                                        .surface
                                        .node(id)
                                        .is_some_and(|n| n.listeners.contains_key(event))
                                };
                                (listens("input"), listens("keydown"))
                            };
                            if composer_epoch.is_none() && listens_input {
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
                            if listens_keydown {
                                this.send(
                                    id,
                                    "keydown",
                                    EventDetail {
                                        key: Some("Enter".into()),
                                        value: composer_epoch.map(|_| json!(value)),
                                        ..Default::default()
                                    },
                                    cx,
                                );
                            }
                        }
                        TextEvent::Cancelled => {
                            pm_mcp::coedit::touch_field(
                                this.runtime.project_id(),
                                &format!("{}/{}", this.publication.panel, id),
                                false,
                            );
                            if this.prepared.node(id).is_some_and(|n| n.cancel_change) {
                                this.change(id, json!(""), cx);
                            }
                        }
                        // Escape in a composer reaches its panel as the key
                        // (the Agent's shortcuts menu, then its popover).
                        TextEvent::Key(key @ ("escape" | "backspace")) => {
                            if this
                                .publication
                                .surface
                                .node(id)
                                .is_some_and(|n| n.listeners.contains_key("keydown"))
                            {
                                let detail = EventDetail {
                                    key: Some(if *key == "escape" { "Escape" } else { key }.into()),
                                    ..Default::default()
                                };
                                this.send(id, "keydown", detail, cx);
                            }
                        }
                        // Up and Down in a number field step its value (Shift:
                        // ten steps) and write it, as a spin field does.
                        TextEvent::Key(key @ ("up" | "down" | "shiftUp" | "shiftDown"))
                            if number =>
                        {
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
                }),
            );
        if let Some(focus) = self.focus.get(&id) {
            let subscription = cx.on_focus(focus, window, move |v, _, cx| {
                if v.prepared.node(id).is_some_and(|n| n.binding.is_some()) {
                    pm_mcp::coedit::touch_field(
                        v.runtime.project_id(),
                        &format!("{}/{}", v.publication.panel, id),
                        true,
                    );
                }
                if let Some(field) = v.fields.get(&id) {
                    field.update(cx, |f, cx| f.begin_editing(cx));
                }
            });
            self.subscriptions.entry(id).or_default().push(subscription);
            let subscription = cx.on_blur(focus, window, move |v, _, _| {
                pm_mcp::coedit::touch_field(
                    v.runtime.project_id(),
                    &format!("{}/{}", v.publication.panel, id),
                    false,
                );
            });
            self.subscriptions.entry(id).or_default().push(subscription);
        }
        // A field the panel opens on demand (a layer's rename, a typed time)
        // takes the keyboard as it appears, its text selected: typing
        // replaces it, and no shortcut fires on the way.
        let focus_on_mount = node
            .props
            .get("metadata")
            .is_some_and(|m| m.get("focusOnMount") == Some(&Value::Bool(true)));
        if view.cancel_change || focus_on_mount {
            field.update(cx, |f, cx| {
                // Escape closes it unchanged (an AppKit field commits on
                // Escape unless told otherwise).
                f.set_escape_cancels(view.cancel_change);
                if let Some(end) = node.props.get("metadata").and_then(|m| m.get("selectionEnd")).and_then(Value::as_u64) {
                    f.begin_editing_selecting_to(end as usize, cx);
                } else { f.begin_editing_selecting_all(cx); }
            });
        }
        self.fields.insert(id, field);
    }
    #[cfg(test)]
    pub fn outline_scroll_for_test(&self) -> f32 { self.natives.outline.scroll_for_test() }
    /// A new tree can have a different window even though the scroll hasn't
    /// moved (e.g. expanding properties switches from a retained outline).
    pub(crate) fn sync_outline_window(&mut self, cx: &mut Context<Self>) {
        self.outline_rows(self.natives.outline.rows.get(), cx);
    }
    #[cfg(test)]
    pub(crate) fn timeline_outline_rows(&self) -> (usize, Vec<String>) {
        let Some(tree) = self
            .prepared
            .semantics
            .get("timeline:tracks")
            .and_then(|id| self.publication.surface.node(*id))
        else {
            return (0, vec![]);
        };
        let (first, _) = self.prepared.node(tree.id).unwrap().outline_window.unwrap();
        let ids = tree
            .children
            .iter()
            .filter_map(|id| {
                let row = self.publication.surface.node(*id)?;
                Some(
                    row.props
                        .get("metadata")?
                        .get("nativeRow")?
                        .as_str()?
                        .to_owned(),
                )
            })
            .collect();
        (first, ids)
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
        let proposed = self
            .outline_waiting
            .as_ref()
            .and_then(|waiting| waiting.outline.as_ref());
        if node.outline_retained && proposed.is_none() {
            return;
        }
        let (id, (first, count)) = (node.id, node.outline_window.unwrap());
        let (from, to) = proposed.map_or((first, first + count), |p| {
            (p.first, p.first + p.rows.len())
        });
        if rows.1 > rows.0
            && (rows.0 < from || rows.1 > to || self.pending_ranges.contains_key(&id))
        {
            self.request_range(
                id,
                RangeRequest::Outline {
                    rows,
                    published: (first, count),
                },
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
    /// Dev preview and headless tests: `event` on the node whose semantic id
    /// is, or ends with, `semantic` (`--open reviewPanel:settings:select:nav.general`).
    #[cfg(any(debug_assertions, test))]
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
    #[cfg(any(debug_assertions, test))]
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
    pub fn test_scroll_offset(&self, id: Id) -> Option<Point<Pixels>> {
        self.scrolls.get(&id).map(|scroll| scroll.offset())
    }
    #[cfg(test)]
    pub(crate) fn test_scroll_state(&self, id: Id) -> Option<(Point<Pixels>, Bounds<Pixels>, Point<Pixels>)> {
        self.scrolls.get(&id).map(|s| (s.offset(), s.bounds(), s.max_offset()))
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
    pub fn wheel_reading(&self) -> Option<f64> {
        self.scrub_shown.map(|(_, value)| value)
    }
    /// The number a numeric field shows now: a scrub's readout, else its
    /// tree's value (tests).
    #[cfg(test)]
    pub fn shown_number(&self, semantic: &str) -> f64 {
        let id = self.prepared.semantics[semantic];
        self.scrub_readout(id)
            .unwrap_or_else(|| self.prepared.node(id).unwrap().number.unwrap().value)
    }
    #[cfg(test)]
    pub fn number_edit_requested(&self, semantic: &str) -> bool {
        self.number_edit_request == Some(self.prepared.semantics[semantic])
    }
    #[cfg(test)]
    pub fn numeric_facts(&self, semantic: &str) -> crate::v4_compile::NumberFacts {
        self.prepared
            .node(self.prepared.semantics[semantic])
            .unwrap()
            .number
            .unwrap()
    }
    #[cfg(test)]
    pub fn number_containing(&self, part: &str) -> Option<String> {
        self.prepared
            .nodes
            .values()
            .find(|n| {
                n.number.is_some()
                    && n.binding.is_some()
                    && n.semantic.as_deref().is_some_and(|s| s.contains(part))
            })
            .and_then(|n| n.semantic.as_ref().map(|s| s.to_string()))
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
        self.take_ready(cx);
    }
    /// Focuses the text field `semantic` as soon as it is mounted; keys typed
    /// before then (`type_early`) land in it, in order, as typing does.
    pub(crate) fn focus_when_ready(&mut self, semantic: &str, cx: &mut Context<Self>) {
        self.ready_text = Some((semantic.into(), String::new(), std::time::Instant::now()));
        self.take_ready(cx);
    }
    /// Text typed while the field asked for is not mounted yet; false when
    /// nothing is waiting for it (a field that never came, after 2 s, waits
    /// no more: keys go where they would).
    pub(crate) fn type_early(&mut self, text: &str) -> bool {
        if self
            .ready_text
            .as_ref()
            .is_some_and(|(_, _, asked)| asked.elapsed() > std::time::Duration::from_secs(2))
        {
            self.ready_text = None;
        }
        match &mut self.ready_text {
            Some((_, typed, _)) => {
                typed.push_str(text);
                true
            }
            None => false,
        }
    }
    fn take_ready(&mut self, cx: &mut Context<Self>) {
        // Asked for by an open that is over (the popover closed before its
        // composer came): never focus a field later, docked or elsewhere.
        if self
            .ready_text
            .as_ref()
            .is_some_and(|(_, _, asked)| asked.elapsed() > std::time::Duration::from_secs(2))
        {
            self.ready_text = None;
        }
        let Some(id) = self
            .ready_text
            .as_ref()
            .and_then(|(semantic, _, _)| self.prepared.semantics.get(semantic).copied())
        else {
            return;
        };
        let Some(field) = self.fields.get(&id).cloned() else {
            return;
        };
        let Some((_, typed, _)) = self.ready_text.take() else {
            return;
        };
        let value = field.update(cx, |f, cx| {
            f.begin_editing(cx);
            if typed.is_empty() {
                return None;
            }
            let value = format!("{}{typed}", f.editing_text());
            f.replace_text(&value, cx);
            Some(value)
        });
        // The panel's model hears it as it hears typing.
        if let Some(value) = value {
            self.send(
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
    /// A panel names the field that should take the keyboard (`focusRequest`,
    /// a field's own id): the field takes it once each time the request
    /// appears or changes, so a search button that opens its field puts the
    /// caret in it and typed letters never fire shortcuts on the way (PM-402).
    fn follow_focus_request(&mut self, cx: &mut Context<Self>) {
        // A text field or a search field (a native control) the request names.
        let takes_text = |v: &Self, id: &Id, cx: &App| {
            v.fields.contains_key(id) || v.controls.get(id).is_some_and(|c| c.read(cx).is_search())
        };
        let request = self
            .publication
            .surface
            .nodes()
            .values()
            .filter_map(|n| n.props.get("focusRequest").and_then(Value::as_str))
            .filter(|r| !r.is_empty())
            .find(|r| {
                self.prepared
                    .semantics
                    .get(*r)
                    .is_some_and(|id| takes_text(self, id, cx))
            })
            .map(str::to_owned);
        if request == self.focus_requested {
            return;
        }
        self.focus_requested = request.clone();
        let Some(id) = request.and_then(|r| self.prepared.semantics.get(&r).copied()) else {
            return;
        };
        // What the person types from now on is for this field, even before it
        // has the keyboard (pm_shell::typing).
        pm_shell::typing::expect();
        if let Some(field) = self.fields.get(&id).cloned() {
            field.update(cx, |f, cx| f.begin_editing(cx));
        } else if let Some(control) = self.controls.get(&id) {
            control.read(cx).focus();
        }
        self.typing_ready(cx);
    }
    /// The field a focus request named has the keyboard: what was typed for
    /// it while it came reaches it now.
    fn typing_ready(&self, cx: &App) {
        let Some(id) = self
            .focus_requested
            .as_ref()
            .and_then(|r| self.prepared.semantics.get(r))
        else {
            return;
        };
        let ready = self.controls.get(id).is_some_and(|c| c.read(cx).focused())
            || self
                .fields
                .get(id)
                .is_some_and(|f| f.read(cx).has_keyboard());
        if ready {
            pm_shell::typing::field_ready();
        }
    }
    /// The text field `semantic` lets go of the keyboard (the editor has it).
    pub(crate) fn blur_semantic(&mut self, semantic: &str, cx: &mut Context<Self>) {
        if let Some((s, _, _)) = &self.ready_text
            && s == semantic
        {
            self.ready_text = None;
        }
        if let Some(field) = self
            .prepared
            .semantics
            .get(semantic)
            .and_then(|id| self.fields.get(id))
            .cloned()
        {
            field.update(cx, |f, cx| f.blur(cx));
        }
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
    /// Whether a native view draws anywhere in the panel.
    pub(crate) fn has_native(&self) -> bool {
        self.prepared.nodes.values().any(|n| n.native.is_some())
    }
    /// Whether a native view (the timeline, the viewer's gizmos) draws
    /// inside this node: its content is not drawn by the panel's scene.
    fn holds_native(&self, id: Id) -> bool {
        let mut stack = vec![id];
        while let Some(at) = stack.pop() {
            let Some(node) = self.prepared.node(at) else {
                continue;
            };
            if node.native.is_some() {
                return true;
            }
            stack.extend(node.children.iter().copied());
        }
        false
    }
    /// Whether a node is the panel's header or inside it.
    fn in_header(&self, mut id: Id) -> bool {
        let Some(header) = self.prepared.header else {
            return false;
        };
        loop {
            if id == header {
                return true;
            }
            match self.prepared.node(id).and_then(|n| n.parent) {
                Some(parent) => id = parent,
                None => return false,
            }
        }
    }
    /// Re-stage AppKit slots whose cached paint did not run this frame (a
    /// slot inside a scroll layer follows the layer's offset).
    pub fn keep_native_slots(&self, window: &Window, cx: &App) {
        self.build_painted.set(self.shown);
        for (id, control) in &self.controls {
            // A control whose row is left out stays where it is: hidden.
            if self.left_out(*id) {
                continue;
            }
            control.read(cx).keep(window);
        }
        for field in self.fields.values() {
            field.read(cx).keep(window);
        }
    }
    fn run(
        &mut self,
        ticket: pm_runtime_v8::Result<pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>>,
        cx: &mut Context<Self>,
    ) {
        self.watch_ticket(ticket, true, cx).detach();
    }

    fn watch_ticket(
        &mut self,
        ticket: pm_runtime_v8::Result<pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>>,
        repaint_on_error: bool,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let result = match ticket {
                Ok(ticket) => ticket.await,
                Err(error) => Err(error),
            };
            if let Err(error) = result
                && error != pm_runtime_v8::Error::Superseded
            {
                let _ = this.update(cx, |v, cx| {
                    v.error = Some(error.to_string());
                    if repaint_on_error {
                        cx.notify();
                    }
                });
            }
        })
    }

    pub fn send(&mut self, id: Id, event: &str, detail: EventDetail, cx: &mut Context<Self>) {
        self.send_with_drop(id, event, detail, None, cx);
    }

    fn send_with_drop(
        &mut self,
        id: Id,
        event: &str,
        mut detail: EventDetail,
        received: Option<gpui_macos::ReceivedDrop>,
        cx: &mut Context<Self>,
    ) {
        if detail.value.as_ref().is_some_and(Value::is_string)
            && self.publication.panel == "agent"
            && let Some(epoch) = self
                .publication
                .surface
                .node(id)
                .and_then(|node| node.props.get("metadata"))
                .and_then(|meta| meta.get("composerEpoch"))
                .and_then(Value::as_u64)
        {
            detail.composer_epoch = Some(epoch);
        }
        #[cfg(test)]
        self.log.push((id, event.into(), detail.clone()));
        if received.is_some() {
            self.runtime
                .set_drop_lookup(crate::media_import::lookup_received_drop);
        }
        let p = &self.publication;
        let semantic = self.prepared.node(id).and_then(|n| n.semantic.as_deref());
        let sent = crate::diag::enabled().then(|| {
            crate::diag::line(
                serde_json::json!({"ui": "send", "panel": p.panel, "event": event,
                "semantic": semantic, "host": gpui_macos::feel::now()}),
            );
            std::time::Instant::now()
        });
        let ticket = self.runtime.queue_activation(
            &p.panel,
            p.generation,
            p.surface.revision(),
            id,
            semantic,
            event,
            detail,
            false,
        );
        self.handling += 1;
        cx.spawn(async move |this, cx| {
            let error = match ticket {
                Ok(ticket) => ticket.await.err().map(|e| e.to_string()),
                Err(error) => Some(error.to_string()),
            };
            // A native project effect takes its own lease before reaching
            // the app queue. A rejected or unhandled event releases this one.
            drop(received);
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
                } else if v.idle() && std::mem::take(&mut v.wake_idle) {
                    cx.notify();
                }
            });
        })
        .detach();
    }
    /// Every event sent to the panel's script has been handled.
    pub fn idle(&self) -> bool {
        self.handling == 0 && self.pending_samples.is_empty()
    }
    /// A publication landing mid-drag must still let the current tree hear
    /// the release. The realm keeps just the latest pending move.
    fn pointer(
        &mut self,
        id: Id,
        event: &'static str,
        detail: EventDetail,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        self.log.push((id, event.into(), detail.clone()));
        let p = &self.publication;
        let ticket = self
            .runtime
            .pointer_event(&p.panel, p.generation, id, event, detail);
        // Reporting a pointer failure follows the existing frame cadence.
        let task = self.watch_ticket(ticket, false, cx);
        if event == "pointermove" {
            self.pointer_move = Some(task);
        } else {
            task.detach();
        }
    }

    fn request_range(
        &mut self,
        id: Id,
        request: RangeRequest,
        detail: EventDetail,
        cx: &mut Context<Self>,
    ) {
        let previous = self
            .pending_ranges
            .get(&id)
            .map(|(request, _)| request)
            .or_else(|| self.sent_ranges.get(&id));
        if previous == Some(&request) {
            return;
        }
        #[cfg(test)]
        self.log.push((id, "rangechange".into(), detail.clone()));
        let p = &self.publication;
        let Ok(ticket) =
            self.runtime
                .pointer_event(&p.panel, p.generation, id, "rangechange", detail)
        else {
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            let sent = ticket.await.is_ok();
            let _ = this.update(cx, |v, _| {
                v.pending_ranges.remove(&id);
                if sent {
                    v.sent_ranges.insert(id, request);
                }
            });
        });
        self.pending_ranges.insert(id, (request, task));
    }
    /// Where an anchor node was last drawn (window points).
    pub fn anchor_bounds(&self, id: Id) -> Option<Bounds<Pixels>> {
        self.anchors.borrow().get(&id).copied()
    }
    /// The display-link readout follows its painted button. If the header
    /// clips that slot, hide the layer too so it cannot cover other controls.
    pub(crate) fn timeline_timecode_bounds(
        &self,
        right: Pixels,
        width: Pixels,
    ) -> Option<Bounds<Pixels>> {
        let (id, transport) = self.prepared.timeline_time?;
        if self.field_open("timeline:time") {
            return None;
        }
        let time = self.anchor_bounds(id)?;
        let transport = self.anchor_bounds(transport)?;
        (time.left() + width <= right.min(transport.right()) && time.size.height > px(0.))
            .then_some(time)
    }

    /// Enabled menu buttons in this surface, in the same coordinates as its anchors.
    pub fn other_menu_triggers(&self, anchor: Id) -> Vec<pm_shell::layout::Rect> {
        let anchors = self.anchors.borrow();
        self.publication
            .surface
            .nodes()
            .values()
            .filter(|node| {
                node.id != anchor
                    && node.props.get("aria-haspopup").and_then(Value::as_str) == Some("menu")
                    && self.prepared.node(node.id).is_some_and(|n| !n.disabled)
            })
            .filter_map(|node| anchors.get(&node.id))
            .map(|bounds| pm_shell::layout::Rect {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                w: f32::from(bounds.size.width),
                h: f32::from(bounds.size.height),
            })
            .collect()
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
    fn open_colour(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        if self.colour.is_some() || self.scrub.is_some() || self.scrub_busy {
            return;
        }
        use crate::{
            colour_model::Color,
            colour_picker::{Edit, Picker},
        };
        let Some(node) = self.publication.surface.node(id) else {
            return;
        };
        let binding = node
            .props
            .get("binding")
            .and_then(Value::as_str)
            .map(SharedString::from);
        let binding = binding.filter(|_| !node.listeners.contains_key("change"));
        let initial = node.props.get("value").cloned().unwrap_or(json!("#808080"));
        let color = node
            .props
            .get("value")
            .and_then(Value::as_str)
            .and_then(|text|Color::parse_draft(text,"Hex",1.))
            .unwrap_or(Color([0.5, 0.5, 0.5, 1.]));
        let tokens = self.table.tokens.clone();
        let source = self.natives.sample_color.borrow().clone();
        let picker = cx.new(|cx| Picker::new(color, tokens, window, source, cx));
        let subscription = cx.subscribe(&picker, move |v, _, edit, cx| match edit {
            Edit::Begin if binding.is_some() => {
                let name = v.undo_name(id);
                v.scrub_ops.push_back(ScrubOp::Begin(name));
                v.scrub_pump(cx);
            }
            Edit::Value(color) => {
                if let Some(binding) = &binding {
                    v.scrub_ops
                        .push_back(ScrubOp::Value(binding.clone(), json!(color)));
                    v.scrub_pump(cx);
                } else {
                    v.change(id, json!(color), cx);
                }
            }
            Edit::Finish(commit) => {
                if !commit && binding.is_none() {v.change(id,initial.clone(),cx);}
                if v.scrub_busy || v.scrub_gesture.is_some() || !v.scrub_ops.is_empty() {
                    v.scrub_ops.push_back(ScrubOp::Finish(*commit));
                    v.scrub_pump(cx);
                }
                v.colour = None;
                let focus = v.focus.get(&id).cloned();
                if let Some(focus) = focus {
                    let handle = v.window;
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, w, cx| focus.focus(w, cx));
                    });
                }
                cx.notify();
            }
            Edit::Begin | Edit::Sample => {}
        });
        self.colour = Some((id, picker, subscription));
        cx.notify();
    }

    /// A local drag sends the newest absolute value. The usual change path
    /// still handles commits and discrete choices.
    fn change_sample(&mut self, id: Id, value: Value, cx: &mut Context<Self>) {
        self.control_sample(id, "change", value, cx);
    }

    fn control_sample(&mut self, id: Id, event: &str, value: Value, cx: &mut Context<Self>) {
        let p = &self.publication;
        let Some(node) = p.surface.node(id) else {
            return;
        };
        let ticket = if node.listeners.contains_key(event) {
            let detail = EventDetail {
                value: Some(value),
                ..Default::default()
            };
            #[cfg(test)]
            self.log.push((id, event.into(), detail.clone()));
            self.runtime.queue_activation(
                &p.panel,
                p.generation,
                p.surface.revision(),
                id,
                self.prepared.node(id).and_then(|n| n.semantic.as_deref()),
                event,
                detail,
                true,
            )
        } else if event == "change"
            && let Some(binding) = node.props.get("binding").and_then(Value::as_str)
        {
            self.runtime.write_sample(&p.panel, binding, value)
        } else {
            return;
        };
        let task = cx.spawn(async move |this, cx| {
            let result = match ticket {
                Ok(ticket) => ticket.await,
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |v, cx| {
                v.pending_samples.remove(&id);
                if let Err(error) = result
                    && error != pm_runtime_v8::Error::Superseded
                {
                    v.error = Some(error.to_string());
                    cx.notify();
                } else if v.idle() && std::mem::take(&mut v.wake_idle) {
                    cx.notify();
                }
            });
        });
        self.pending_samples.insert(id, task);
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
        if self.publication.panel == "timeline"
            && let Some(row) = self
                .prepared
                .node(id)
                .and_then(|node| node.semantic.as_deref())
                .and_then(|id| id.strip_prefix("timeline:disclosure:"))
            && self.natives.outline.toggle_disclosure(row, cx)
        {
            return;
        }
        if self.publication.panel == "agent"
            && self.prepared.semantics.get("agent-send") == Some(&id)
            && let Some(editor) = self.prepared.semantics.get("agent-composer-text")
            && let Some(field) = self.fields.get(editor)
        {
            let Some(text) = field.update(cx, |field, _| field.composer_submission()) else {
                return;
            };
            let epoch = self
                .publication
                .surface
                .node(*editor)
                .and_then(|node| node.props.get("metadata"))
                .and_then(|meta| meta.get("composerEpoch"))
                .and_then(Value::as_u64);
            self.send(
                id,
                "click",
                EventDetail {
                    value: Some(json!(text)),
                    composer_epoch: epoch,
                    ..detail
                },
                cx,
            );
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
            Some(Component::PopUpButton)
                if node.props.get("metadata").and_then(|m| m.get("manualMenu"))
                    == Some(&Value::Bool(true)) =>
            {
                self.send(id, "click", detail, cx);
            }
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
                self.open_colour(id, window, cx);
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
    /// The nearest focusable node holding `id` (itself first) takes the
    /// keyboard, as a click focuses it on the web and in AppKit. Text fields
    /// and native controls keep their own focus handling.
    fn focus_pressed(&self, id: Id, window: &mut Window, cx: &mut App) {
        let mut at = Some(id);
        while let Some(node) = at {
            if self.fields.contains_key(&node) || self.controls.contains_key(&node) {
                return;
            }
            if let Some(focus) = self.focus.get(&node) {
                if !focus.is_focused(window) {
                    focus.focus(window, cx);
                }
                return;
            }
            at = self.prepared.node(node).and_then(|n| n.parent);
        }
    }
    fn press(&mut self, id: Id, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let layer = self
            .prepared
            .node(id)
            .and_then(|n| n.timeline_layer_drag.clone());
        if let Some(layer) = &layer {
            self.natives.outline.press_layer(layer, event.position, cx);
        }
        let button = button_code(event.button);
        self.capture = Some(Capture {
            target: id,
            button,
            origin: event.position,
            layer_drag: layer.is_some(),
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
    fn wheel_finish(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some((id, _, bound, start)) = self.wheel_scrub.take() else {
            return;
        };
        if self.finish_gain_scrub(id, commit, cx) {
            return;
        }
        if !commit {
            // As a cancelled pointer scrub: the number it started from, shown
            // and in the field now, kept through the rollback's completion
            // (the rollback's tree may be the one already shown, which is not
            // published again), and no queued preview after it.
            if let Some(number) = self.prepared.node(id).and_then(|n| n.number) {
                if let Some(field) = self.fields.get(&id) {
                    let text = number.text(start);
                    field.update(cx, |f, cx| f.set_value(&text, cx));
                }
            }
            if bound {
                while matches!(self.scrub_ops.back(), Some(ScrubOp::Value(..))) {
                    self.scrub_ops.pop_back();
                }
                self.scrub_shown = Some((id, start));
            } else {
                self.scrub_shown = None;
                self.change(id, json!(start), cx);
            }
            cx.notify();
        }
        if bound {
            self.scrub_ops.push_back(ScrubOp::Finish(commit));
            self.scrub_pump(cx);
        }
    }
    fn begin_gain_scrub(
        &mut self,
        binding: Option<&SharedString>,
        value: f64,
        name: Option<&str>,
    ) -> bool {
        let Some(binding) = binding.filter(|_| self.natives.gain_preview.borrow().is_some()) else {
            return false;
        };
        // Never waits on the inspector's worker: while it is busy, the drag
        // is an ordinary scrub (no live gain preview), still one undo step.
        let Some(mut request) =
            self.runtime
                .try_gain_scrub_request(&self.publication.panel, binding.as_ref(), value)
        else {
            return false;
        };
        if let pm_kernel::Call::Transact { label, .. } = &mut request.call {
            *label = Some(name.unwrap_or("Change Property").into());
        }
        self.gain_scrub = Some(GainScrub {
            gesture: GainPreview::gesture(),
            binding: binding.clone(),
            value,
            request,
        });
        true
    }

    fn sample_gain_scrub(&mut self, value: f64, cx: &mut Context<Self>) -> bool {
        let Some(scrub) = &mut self.gain_scrub else {
            return false;
        };
        if value == scrub.value {
            return true;
        }
        let pm_kernel::Call::Transact {
            body: pm_kernel::WriteBody::Commands(commands),
            ..
        } = &mut scrub.request.call
        else {
            unreachable!("gain scrub resolves a command transaction");
        };
        let time = self
            .natives
            .gain_preview
            .borrow()
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .map(|controller| controller.read(cx).time());
        for command in &mut *commands {
            match command {
                pm_kernel::Command::Property(pm_kernel::PropertyCommand::Set(set)) => {
                    set.value = pm_model::Value::Number(value);
                    if let Some(time) = time {
                        set.time = time;
                    }
                }
                pm_kernel::Command::Content(pm_kernel::ContentCommand::Set {
                    patch,
                    time: at,
                    ..
                }) => {
                    let key = if patch.get("gain").is_some() {
                        "gain"
                    } else {
                        "audioGain"
                    };
                    patch.insert(key, pm_model::Value::Number(value));
                    if let Some(time) = time {
                        *at = time;
                    }
                }
                _ => unreachable!("gain scrub resolves only gain commands"),
            }
        }
        scrub.value = value;
        self.natives.send_gain_preview(
            GainPreview::Sample {
                gesture: scrub.gesture,
                base_revision: None,
                commands: commands.clone(),
            },
            cx,
        );
        true
    }

    fn finish_gain_scrub(&mut self, id: Id, commit: bool, cx: &mut Context<Self>) -> bool {
        let Some(scrub) = self.gain_scrub.take() else {
            return false;
        };
        self.scrub_shown = None;
        if !commit {
            self.restore_gain_field(id, cx);
            let natives = self.natives.clone();
            cx.defer(move |cx| {
                natives.send_gain_preview(
                    GainPreview::Finish {
                        gesture: scrub.gesture,
                        revision: None,
                    },
                    cx,
                )
            });
            cx.notify();
            return true;
        }
        // Check the lease again and commit the exact last sample, including
        // the playhead time that sample used for an animated channel.
        let pending = self
            .runtime
            .gain_scrub_request(&self.publication.panel, scrub.binding.as_ref(), scrub.value)
            .map(|_| self.runtime.commit_gain_scrub(scrub.request));
        let natives = self.natives.clone();
        let gesture = scrub.gesture;
        cx.spawn(async move |this, cx| {
            let result = match pending {
                Some(pending) => match pending.await {
                    Some(Ok(ack)) => match ack.reply {
                        pm_kernel::Reply::Throw(error) => Err(error.to_string()),
                        _ => Ok(ack.revision),
                    },
                    Some(Err(error)) => Err(error.to_string()),
                    None => Err("Kernel closed".into()),
                },
                None => Err("The gain control is no longer active".into()),
            };
            let revision = result.as_ref().ok().copied();
            cx.update(|cx| {
                natives.send_gain_preview(GainPreview::Finish { gesture, revision }, cx)
            });
            let _ = this.update(cx, |view, cx| {
                if let Err(error) = result {
                    view.error = Some(error);
                    view.restore_gain_field(id, cx);
                }
                cx.notify();
            });
        })
        .detach();
        true
    }

    fn restore_gain_field(&self, id: Id, cx: &mut Context<Self>) {
        if let Some(number) = self.prepared.node(id).and_then(|node| node.number)
            && let Some(field) = self.fields.get(&id)
        {
            field.update(cx, |f, cx| f.set_value(&number.text(number.value), cx));
        }
    }

    fn wheel_number(
        &mut self,
        id: Id,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use pm_surface::kit::{HORIZONTAL_SCRUB_REST, HorizontalScrub};
        if !window.is_window_active() {
            return;
        }
        if event.touch_phase == TouchPhase::Started {
            self.wheel_lifted = false;
        }
        if matches!(event.touch_phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.wheel_lifted = true;
            if self.wheel_scrub.is_some() {
                window.prevent_default();
                cx.stop_propagation();
                if event.touch_phase == TouchPhase::Cancelled {
                    self.wheel_finish(false, cx);
                }
            }
            return;
        }
        let delta = match event.delta {
            ScrollDelta::Pixels(p) => (-f32::from(p.x) as f64, -f32::from(p.y) as f64),
            ScrollDelta::Lines(p) => (-p.x as f64 * 16., -p.y as f64 * 16.),
        };
        if self.wheel_lifted && matches!(event.delta, ScrollDelta::Pixels(_)) {
            if delta.0.abs() > delta.1.abs() || self.wheel_scrub.is_some() {
                window.prevent_default();
                cx.stop_propagation();
            }
            return;
        }
        if self.scrub.is_some() {
            return;
        }
        let Some(node) = self.prepared.node(id) else {
            return;
        };
        let Some(n) = node.number else {
            return;
        };
        if node.disabled
            || self
                .fields
                .get(&id)
                .is_some_and(|f| f.read(cx).is_editing())
        {
            return;
        }
        let binding = node.binding.clone().filter(|_| !node.local_change);
        let active = self
            .wheel_scrub
            .as_ref()
            .filter(|(target, _, _, _)| *target == id);
        if !active.map_or_else(
            || HorizontalScrub::new(n.value).accepts(delta.0, delta.1, event.modifiers.control),
            |(_, w, _, _)| w.accepts(delta.0, delta.1, event.modifiers.control),
        ) {
            return;
        }
        window.prevent_default();
        cx.stop_propagation();
        if active.is_none() {
            self.wheel_finish(true, cx);
            self.wheel_scrub = Some((
                id,
                HorizontalScrub::new(n.value),
                binding.is_some(),
                n.value,
            ));
            let name = self.undo_name(id);
            if !self.begin_gain_scrub(binding.as_ref(), n.value, name.as_deref()) && binding.is_some() {
                self.scrub_ops.push_back(ScrubOp::Begin(name));
            }
        }
        if delta.0 == 0. {
            return;
        }
        let value = self.wheel_scrub.as_mut().unwrap().1.sample(
            delta.0,
            n.step,
            n.speed,
            event.modifiers.shift,
            event.modifiers.alt,
            (n.min, n.max),
        );
        self.scrub_shown = Some((id, value));
        if !self.sample_gain_scrub(value, cx) {
            if let Some(binding) = binding {
                self.scrub_ops
                    .push_back(ScrubOp::Value(binding, json!(value)));
                self.scrub_pump(cx);
            } else {
                self.change(id, json!(value), cx);
            }
        }
        if let Some(field) = self.fields.get(&id) {
            field.update(cx, |f, cx| f.set_value(&n.text(value), cx));
        }
        self.wheel_tick += 1;
        let tick = self.wheel_tick;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HORIZONTAL_SCRUB_REST).await;
            let _ = this.update(cx, |v, cx| {
                if v.wheel_tick == tick {
                    v.wheel_finish(true, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }
    fn scrub_readout(&self, id: Id) -> Option<f64> {
        self.scrub_shown
            .filter(|(target, _)| *target == id)
            .map(|(_, value)| value)
            .or_else(|| {
                self.scrub_settled
                    .filter(|(target, _, _)| *target == id)
                    .map(|(_, value, _)| value)
            })
    }

    fn scrub_press(
        &mut self,
        id: Id,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.wheel_finish(true, cx);
        let Some(node) = self.prepared.node(id) else {
            return;
        };
        let Some(mut number) = node.number.filter(|_| !node.disabled) else {
            return;
        };
        if let Some(value) = self.scrub_readout(id) {
            number.value = value;
        }
        // A press must not focus (and so open) the text field before a drag decides.
        window.prevent_default();
        self.scrub = Some(ScrubDrag {
            id,
            start: event.position,
            number,
            motion: pm_surface::kit::NumericMotion::new(
                f64::from(event.position.x),
                number.value,
            ),
            infinite: crate::infinite_drag::InfiniteDrag::new(),
            binding: node.binding.clone().filter(|_| !node.local_change),
            active: false,
            grade: None,
            local_commit: node.commit_on_release.then(|| {
                let value = json!(number.value);
                (value.clone(), value)
            }),
        });
    }
    fn scrub_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.scrub.as_ref().is_some_and(|d| d.grade.is_some()) {
            return self.grade_move(event, cx);
        }
        self.scrub_at(event.position.x, event.modifiers, cx);
    }

    fn scrub_at(&mut self, x: Pixels, modifiers: Modifiers, cx: &mut Context<Self>) {
        let threshold = self.table.tokens.metrics.drag_threshold;
        let name = self
            .scrub
            .as_ref()
            .filter(|d| !d.active)
            .and_then(|d| self.undo_name(d.id));
        let Some(drag) = &mut self.scrub else {
            return;
        };
        let n = drag.number;
        let sampled = if let Some(dx) = drag.infinite.take_delta_x() {
            drag.motion.sample_delta(
                dx,
                threshold as f64,
                n.step,
                n.speed,
                modifiers.shift,
                modifiers.alt,
                (n.min, n.max),
            )
        } else {
            drag.motion.sample(
                f64::from(x),
                threshold as f64,
                n.step,
                n.speed,
                modifiers.shift,
                modifiers.alt,
                (n.min, n.max),
            )
        };
        let Some(value) = sampled else {
            return;
        };
        let began = !drag.active;
        drag.active = true;
        if began {
            drag.infinite.engage();
        }
        let id = drag.id;
        if !began && self.scrub_shown == Some((id, value)) {
            return;
        }
        let binding = drag.binding.clone();
        let deferred = if let Some((_, last)) = &mut drag.local_commit {
            *last = json!(value);
            true
        } else {
            false
        };
        let live_gain = (began && self.begin_gain_scrub(binding.as_ref(), n.value, name.as_deref()))
            || self.gain_scrub.is_some();
        self.scrub_shown = Some((id, value));
        if live_gain {
            self.sample_gain_scrub(value, cx);
        } else {
            match binding {
                Some(binding) => {
                    if began {
                        self.scrub_ops.push_back(ScrubOp::Begin(name));
                    }
                    self.scrub_ops
                        .push_back(ScrubOp::Value(binding, json!(value)));
                    self.scrub_pump(cx);
                }
                None if deferred => self.control_sample(id, "input", json!(n.text(value)), cx),
                None => self.change_sample(id, json!(value), cx),
            }
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
        if self.finish_gain_scrub(drag.id, commit, cx) {
            return;
        }
        if let Some((from, live)) = drag.local_commit {
            if !drag.active && drag.grade.is_none() {
                if commit && let Some(field) = self.fields.get(&drag.id) {
                    field.update(cx, |field, cx| field.begin_editing(cx));
                }
                return;
            }
            self.scrub_shown = None;
            let changed = from != live;
            let value = if commit { live } else { from };
            if drag.grade.is_some()
                && let Ok(curve) = serde_json::from_value::<[f64; 4]>(value.clone())
            {
                self.ease
                    .pending
                    .borrow_mut()
                    .insert(drag.id, (curve, std::time::Instant::now()));
            }
            if !commit {
                if let Some(number) = value.as_f64()
                    && let Some(field) = self.fields.get(&drag.id)
                {
                    let text = drag.number.text(number);
                    field.update(cx, |field, cx| field.set_value(&text, cx));
                }
                let observed = if drag.grade.is_none() {
                    value
                        .as_f64()
                        .map(|n| json!(drag.number.text(n)))
                        .unwrap_or(value)
                } else {
                    value
                };
                self.pointer(
                    drag.id,
                    "input",
                    EventDetail {
                        value: Some(observed),
                        ..Default::default()
                    },
                    cx,
                );
            } else if changed {
                self.pointer(
                    drag.id,
                    "change",
                    EventDetail {
                        value: Some(value),
                        ..Default::default()
                    },
                    cx,
                );
            }
            cx.notify();
            return;
        }
        if drag.active {
            if !commit && drag.grade.is_none() {
                self.scrub_shown = Some((drag.id, drag.number.value));
                if let Some(field) = self.fields.get(&drag.id) {
                    let text = drag.number.text(drag.number.value);
                    field.update(cx, |f, cx| f.set_value(&text, cx));
                }
            }
            if drag.binding.is_some() {
                if !commit {
                    while matches!(self.scrub_ops.back(), Some(ScrubOp::Value(..))) {
                        self.scrub_ops.pop_back();
                    }
                }
                self.scrub_ops.push_back(ScrubOp::Finish(commit));
                self.scrub_pump(cx);
            } else if drag.grade.is_none() {
                if !commit {
                    self.change_sample(drag.id, json!(drag.number.value), cx);
                }
                self.scrub_shown = None;
            }
            cx.notify();
        } else if drag.grade.is_some() {
            // A press on a grading control that never moved changes nothing.
        } else if commit {
            #[cfg(test)]
            {
                self.number_edit_request = Some(drag.id);
            }
            if let Some(field) = self.fields.get(&drag.id) {
                field.update(cx, |f, cx| f.begin_editing(cx));
            }
        }
    }
    /// Escape during a scrub restores the value through the kernel gesture.
    pub fn cancel_gesture(&mut self, cx: &mut Context<Self>) -> bool {
        if self.wheel_scrub.is_some() {
            self.wheel_finish(false, cx);
            return true;
        }
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
        let finishing = matches!(op, ScrubOp::Finish(_));
        crate::interaction_work::spawn!(cx, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let pending = match (op, gesture) {
                        (ScrubOp::Begin(label), _) => runtime.scrub_begin(&panel, label.as_deref()),
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
                        .map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.scrub_busy = false;
                let finished = finishing && result.as_ref().is_ok_and(|ack|
                    !matches!(ack.reply, pm_kernel::Reply::Throw(_))
                );
                if finishing {
                    v.scrub_gesture = None;
                }
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
                if v.scrub_ops.is_empty() && v.scrub.is_none() && v.wheel_scrub.is_none() && v.colour.is_none() {
                    v.scrub_gesture = None;
                    if let Some((id, value)) = v.scrub_shown.take() {
                        v.scrub_settled = finished.then_some((id, value, v.latest));
                    }
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
            move |_, _, window, cx| {
                if let Some(view) = weak.upgrade()
                    && view.read(cx).scrub.as_ref().is_some_and(|d| d.active && d.grade.is_none())
                {
                    window.set_window_cursor_style(gpui::CursorStyle::ResizeLeftRight);
                }
                let wheel = weak.clone();
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = wheel.update(cx, |v, _| {
                            if event.touch_phase == TouchPhase::Started {
                                v.wheel_lifted = false;
                            }
                            if matches!(
                                event.touch_phase,
                                TouchPhase::Ended | TouchPhase::Cancelled
                            ) {
                                v.wheel_lifted = true;
                            }
                        });
                    }
                });
                let moved = weak.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let mut row_move = None;
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
                        // A row reorder is native. Forward its captured moves before
                        // generic panel capture can consume them; native listener
                        // ordering may change when a cached surface is repainted.
                        if capture.layer_drag {
                            row_move = Some(v.natives.outline.clone());
                            cx.stop_propagation();
                            return;
                        }
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
                    // Scrolling updates this SurfaceView through Outline::scrolled.
                    // Release its entity borrow before forwarding to the timeline.
                    if let Some(outline) = row_move {
                        outline.move_layer(event, window, cx);
                    }
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
                    if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                        return;
                    }
                    let _ = released.update(cx, |v, cx| {
                        if v.scrub.as_ref().is_some_and(|drag| drag.grade.is_none()) {
                            v.scrub_at(event.position.x, event.modifiers, cx);
                        }
                        v.release_pointer(event.position, event.click_count, event.modifiers, cx)
                    });
                });
            },
        )
        .absolute()
        .inset_0()
    }
    /// The keyboard is on a node of this view that hears keys, itself or
    /// through what holds it (a Project card a click focused, in a grid that
    /// listens): its Space is its own (Quick Look), not Play.
    pub fn focused_takes_keys(&self, window: &Window) -> bool {
        let Some(mut at) = self
            .focus
            .iter()
            .find_map(|(id, focus)| focus.is_focused(window).then_some(*id))
        else {
            return false;
        };
        while let Some(node) = self.prepared.node(at) {
            if node.events.key_down {
                return true;
            }
            let Some(parent) = node.parent else { break };
            at = parent;
        }
        false
    }
    /// The button came up in the window while a scrub or a captured press of
    /// this view still waits for it: its own listener never heard the
    /// release (the panel was drawn again while held). It ends now.
    pub fn release_unheard(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let held = self.scrub.is_some() || self.capture.is_some();
        if held {
            self.release_pointer(window.mouse_position(), 1, Modifiers::none(), cx);
        }
        held
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
        if capture.layer_drag {
            self.natives.outline.release_layer(cx);
        }
        if self
            .prepared
            .node(capture.target)
            .is_some_and(|n| n.events.pointer_up)
        {
            let detail = self.detail(position, Some(capture.button), 0, click_count, modifiers);
            self.pointer(capture.target, "pointerup", detail, cx);
        }
    }
    pub(crate) fn copy_selection(&self, window: &Window, cx: &mut App) -> bool {
        let selected = self.quote_selection.borrow();
        let Some(owner) = selected.owner else {
            return false;
        };
        if !crate::v4_quote::cite_context(
            self.publication.panel == "agent",
            self.focus.get(&owner).is_some_and(|f| f.is_focused(window)),
            selected.range(),
        ) {
            return false;
        }
        let text = selected.text(owner);
        let Some(quote) = text.get(selected.range()) else {
            return false;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(quote.to_owned()));
        true
    }
    pub(crate) fn cite_selection(&mut self, window: &Window, cx: &mut Context<Self>) -> bool {
        let selected = self.quote_selection.borrow();
        let Some(owner) = selected.owner else {
            return false;
        };
        if !crate::v4_quote::cite_context(
            self.publication.panel == "agent",
            self.focus.get(&owner).is_some_and(|f| f.is_focused(window)),
            selected.range(),
        ) {
            return false;
        }
        drop(selected);
        self.publish_quote(owner, cx);
        self.send(
            owner,
            "keydown",
            EventDetail {
                key: Some(".".into()),
                meta_key: Some(true),
                shift_key: Some(true),
                ..Default::default()
            },
            cx,
        );
        true
    }
    fn quote_owner(&self, mut id: Id) -> Option<Id> {
        if self.publication.panel != "agent" {
            return None;
        }
        loop {
            if self
                .publication
                .surface
                .node(id)?
                .listeners
                .contains_key("click")
            {
                return None;
            }
            if self
                .publication
                .surface
                .node(id)?
                .props
                .get("metadata")
                .and_then(|m| m.get("quoteSource"))
                .is_some()
            {
                return Some(id);
            }
            id = self.prepared.node(id)?.parent?;
        }
    }
    fn quote_plain<'a>(&'a self, id: Id, display: &'a str) -> &'a str {
        let mut at = Some(id);
        while let Some(id) = at {
            let metadata = self.publication.surface.node(id).and_then(|n| n.props.get("metadata"));
            if let Some(plain) = metadata.and_then(|m| m.get("quoteText")).and_then(Value::as_str) {
                return plain;
            }
            if metadata.and_then(|m| m.get("quoteSource")).is_some() {
                break;
            }
            at = self.prepared.node(id).and_then(|n| n.parent);
        }
        display
    }
    fn quote_blocks(&self, root: Id, out: &mut Vec<(Id, String)>) {
        if self
            .publication
            .surface
            .node(root)
            .is_some_and(|n| n.listeners.contains_key("click"))
        {
            return;
        }
        let Some(n) = self.prepared.node(root) else {
            return;
        };
        if let Some(rich) = &n.rich {
            out.push((root, self.quote_plain(root, &rich.text).to_owned()));
        } else if let Some(text) = &n.text {
            out.push((root, self.quote_plain(root, text).to_owned()));
        }
        for child in &n.children {
            self.quote_blocks(*child, out);
        }
    }
    fn quote_range(&self, id: Id) -> Option<std::ops::Range<usize>> {
        let selected = self.quote_selection.borrow();
        let block = selected.blocks.get(&id)?;
        if selected.owner != Some(block.owner) {
            return None;
        }
        let range = selected.range();
        let node = self.prepared.node(id)?;
        let current: &str = node
            .rich
            .as_ref()
            .map(|r| r.text.as_ref())
            .or_else(|| node.text.as_deref())?;
        let plain = self.quote_plain(id, current);
        let indices = crate::v4_quote::Indices::new(plain, current);
        let mut start = range.start.saturating_sub(block.offset).min(plain.len());
        let mut end = range.end.saturating_sub(block.offset).min(plain.len());
        while !plain.is_char_boundary(start) {
            start -= 1;
        }
        while !plain.is_char_boundary(end) {
            end -= 1;
        }
        (start < end).then(|| indices.display(start)..indices.display(end))
    }
    fn plain_or_quote_text(&self, id: Id, text: &SharedString) -> AnyElement {
        if self.quote_owner(id).is_some() {
            self.quote_text(id, StyledText::new(text.clone()), text.to_string())
        } else {
            text.clone().into_any_element()
        }
    }
    fn quote_link(&self, owner: Id, position: Point<Pixels>) -> Option<String> {
        let selection = self.quote_selection.borrow();
        selection
            .blocks
            .iter()
            .filter(|(_, block)| block.owner == owner)
            .find_map(|(id, block)| {
                let rich = self.prepared.node(*id)?.rich.as_ref()?;
                let at = block.layout.index_for_position(position).ok()?;
                rich.links
                    .iter()
                    .find(|(range, _)| range.contains(&at))
                    .map(|(_, href)| href.clone())
            })
    }

    fn quote_text(&self, id: Id, mut text: StyledText, plain: String) -> AnyElement {
        let Some(owner) = self.quote_owner(id) else {
            return text.into_any_element();
        };
        if self.prepared.node(id).is_none_or(|n| n.rich.is_none()) {
            if let Some(range) = self.quote_range(id) {
                text = text.with_highlights([(
                    range,
                    HighlightStyle {
                        color: Some(gpui::rgb_to_hsla(rgb(self.table.tokens.colors.selected_text))),
                        ..Default::default()
                    },
                )]);
            }
        }
        let mut blocks = Vec::new();
        self.quote_blocks(owner, &mut blocks);
        let mut offset = 0;
        for (block, content) in blocks {
            if block == id {
                break;
            }
            offset += content.len() + 1;
        }
        crate::v4_quote::Selectable {
            block: crate::v4_quote::Block {
                owner,
                offset,
                indices: crate::v4_quote::Indices::new(self.quote_plain(id, &plain), &plain),
                text: self.quote_plain(id, &plain).to_owned(),
                layout: text.layout().clone(),
            },
            text,
            id,
            selection: self.quote_selection.clone(),
            color: gpui::rgb_to_hsla(rgb(self.table.tokens.colors.selected_fill)),
        }
        .into_any_element()
    }
    fn publish_quote(&mut self, owner: Id, cx: &mut Context<Self>) {
        let selected = self.quote_selection.borrow();
        let text = selected.text(owner);
        let range = selected.range();
        let mut start = range.start.min(text.len());
        let mut end = range.end.min(text.len());
        while !text.is_char_boundary(start) {
            start -= 1;
        }
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let utf16 = (
            text[..start].encode_utf16().count(),
            text[..end].encode_utf16().count(),
        );
        let position = selected.end_position().and_then(|caret| {
            let source = self.anchors.borrow().get(&owner).copied()?;
            let mut viewport = source;
            let mut ancestor = owner;
            while let Some(parent) = self.prepared.node(ancestor).and_then(|node| node.parent) {
                if let Some(scroll) = self.scrolls.get(&parent) {
                    viewport = scroll.bounds();
                    break;
                }
                ancestor = parent;
            }
            Some(crate::v4_quote::pill_origin(
                caret,
                source,
                viewport,
                size(
                    px(self.table.tokens.metrics.hit * 2.),
                    px(self.table.tokens.metrics.hit),
                ),
            ))
        });
        drop(selected);
        self.send(
            owner,
            "select",
            EventDetail {
                value: Some(json!(text)),
                selection: Some(utf16),
                x: position.map(|p| f32::from(p.x) as f64),
                y: position.map(|p| f32::from(p.y) as f64),
                ..Default::default()
            },
            cx,
        );
    }
    fn element(
        &self,
        id: Id,
        window: &mut Window,
        cx: &mut Context<Self>,
        skip: Option<Id>,
    ) -> AnyElement {
        // The render recurses here once per level of the tree: this frame
        // holds only the children loop. What a node is before and after its
        // children is built in frames of their own (`element_open`,
        // `element_close`): in one frame, a debug build gave each level
        // 180 KB, and ten levels with a symbol drawn under them overflowed a
        // 2 MB thread.
        let (element, mut children, shown) = match self.element_open(id, window, cx, skip) {
            Ok(open) => open,
            Err(done) => return done,
        };
        let Some(node) = self.prepared.node(id) else {
            return div().into_any_element();
        };
        // A standalone surface has no native timeline scene to supply a range.
        let retained_rows = if node.outline_retained {
            self.natives.outline.scene.borrow().as_ref().map(|scene| {
                let (start, end) = self.natives.outline.rows.get();
                (start..end).filter_map(|index| scene.span(index))
                    .map(|(row, _, _)| row.id.clone())
                    .collect::<std::collections::BTreeSet<_>>()
            })
        } else { None };
        for child in node.children.iter().take(shown).filter(|child| {
            retained_rows.as_ref().is_none_or(|rows| {
                self.prepared.node(**child).and_then(|node| node.semantic.as_ref())
                    .and_then(|id| id.strip_prefix("timeline:row:"))
                    .is_some_and(|id| rows.contains(id))
            })
        }) {
            if !self.hover_shown(*child) {
                continue;
            }
            if let Some(stand_in) = self.cull.stand_in(*child) {
                children.push(stand_in);
                continue;
            }
            match self.prepared.node(*child) {
                Some(c) if c.is_plain_text => {
                    if let Some(text) = &c.text {
                        children.push(self.plain_or_quote_text(c.id, text));
                    }
                }
                _ if node.fixed_items => {
                    let size = node.virtual_window.map_or(0., |g| g.item_size) as f32;
                    children.push(
                        div()
                            .flex_none()
                            .h(px(size))
                            .flex()
                            .flex_col()
                            .child(self.element(*child, window, cx, skip))
                            .into_any_element(),
                    );
                }
                _ => children.push(self.element(*child, window, cx, skip)),
            }
        }
        if node.hover_group {
            children.push(self.hover_probe(id, cx));
        }
        self.element_close(id, element, children, window, cx)
    }
    /// A hover group's hitbox, recorded as it paints for the next render to
    /// test against the pointer. A layout that moved the group under a still
    /// pointer (rows scrolled, a row added above) changes what it covers:
    /// then the view draws once more with the groups the pointer is over.
    fn hover_probe(&self, id: Id, cx: &mut Context<Self>) -> AnyElement {
        let (painted, view) = (self.painted_groups.clone(), cx.entity().downgrade());
        let rendered = self.hovered_groups.contains(&id);
        // A scroll moves the rows under a still pointer: the draw that ends
        // it finds the group (PM-732).
        let scrolling = self.scrolling();
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal),
            move |_, hitbox, window, cx| {
                painted.borrow_mut().insert(id, hitbox.id);
                if !scrolling && hitbox.is_hovered(window) != rendered {
                    let view = view.clone();
                    cx.defer(move |cx| {
                        let _ = view.update(cx, |_, cx| cx.notify());
                    });
                }
            },
        )
        .absolute()
        .inset_0()
        .into_any_element()
    }
    /// The hover group the pointer is over, by its semantic id (tests).
    #[cfg(test)]
    pub(crate) fn hovered_row_for_test(&self) -> Option<String> {
        let id = *self.hovered_groups.iter().next()?;
        self.prepared
            .node(id)?
            .semantic
            .as_ref()
            .map(|s| s.to_string())
    }
    /// How far the panel's own scrollers have moved, all summed (tests: a
    /// scroll case proves it moved something).
    #[cfg(test)]
    pub(crate) fn scroll_offsets(&self) -> f32 {
        self.scrolls
            .values()
            .map(|h| f32::from(h.offset().x).abs() + f32::from(h.offset().y).abs())
            .sum()
    }
    /// A `hoverOnly` node shows while the hover group holding it is hovered,
    /// a `hoverHide` one until then; every other node always.
    fn hover_shown(&self, id: Id) -> bool {
        let Some(shows) = self.prepared.node(id).and_then(|n| n.hover_shows) else {
            return true;
        };
        let mut at = self.prepared.node(id).and_then(|n| n.parent);
        while let Some(parent) = at {
            let Some(node) = self.prepared.node(parent) else {
                break;
            };
            if node.hover_group {
                return self.hovered_groups.contains(&parent) == shows;
            }
            at = node.parent;
        }
        !shows
    }
    /// Draw the published search field and kit switch when no AppKit slot exists.
    fn offscreen_control(
        &self,
        node: &NodeView,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.appkit
            || !matches!(node.component, Some(Component::Switch | Component::TextField))
        {
            return None;
        }
        let t = &self.table.tokens;
        let props = &self.publication.surface.node(node.id)?.props;
        if node.component == Some(Component::Switch) {
            let kit = pm_shell::kit::Kit::new(t, &self.motion, |v| &mut v.motion, window, cx);
            let on = props.get("value").and_then(Value::as_bool) == Some(true);
            return Some(
                kit.switch(format!("v4-switch-{}", node.id), on)
                    .into_any_element(),
            );
        }
        if props.get("metadata").and_then(|m| m.get("search")) != Some(&Value::Bool(true)) {
            return None;
        }
        let label = self.field_value_or_placeholder(node)?;
        let has_value = props
            .get("value")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty());
        let ink = if has_value && !node.disabled {
            t.colors.text
        } else {
            t.colors.muted
        };
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(t.metrics.gap))
                .w_full()
                .child(pm_shell::symbol::symbol(
                    "magnifyingglass",
                    t.metrics.symbol_inline,
                    pm_shell::symbol::Weight::Regular,
                    t.colors.secondary,
                    window,
                ))
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_color(rgb(ink))
                        .child(label),
                )
                .into_any_element(),
        )
    }

    /// A node's element up to its children, and the children that come
    /// before them; `Err` when the node is all there is (a control, a field,
    /// nothing).
    #[inline(never)]
    fn element_open(
        &self,
        id: Id,
        window: &mut Window,
        cx: &mut Context<Self>,
        skip: Option<Id>,
    ) -> Result<(Box<Stateful<Div>>, Vec<AnyElement>, usize), AnyElement> {
        if skip == Some(id) {
            return Err(div().into_any_element());
        }
        let Some(node) = self.prepared.node(id) else {
            return Err(div().into_any_element());
        };
        if node.hidden {
            return Err(div().into_any_element());
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
            if self.publication.surface.node(id).is_some_and(|n| {
                n.props
                    .get("metadata")
                    .is_some_and(|m| m["manualMenu"] == true)
            }) {
                slot.style().size.width = node.style.size.width.clone();
                slot.style().min_size.width = node.style.min_size.width.clone();
            }
            if let Some(focus) = self.focus.get(&id) {
                // No focus ring on a control's slot (owner decision,
                // 2026-11-14): a field keeps its own caret and selection.
                let radius = px(self.table.tokens.metrics.control_radius);
                let tab = node.tab_index.unwrap_or(0);
                let handle = &focus.clone().tab_stop(tab >= 0).tab_index(tab.max(0));
                let control = control.clone();
                slot = slot
                    .track_focus(handle)
                    .tab_index(tab.max(0))
                    .rounded(radius)
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
            return Err(slot
                .child(child)
                .children(self.probe(id))
                .into_any_element());
        }
        if let Some(field) = self.fields.get(&id) {
            return Err(self.field_element(node, field, cx));
        }
        let mut element = div().id(("v4", id));
        *element.style() = node.style.clone();
        if node.hover_group
            && self.hovered_groups.contains(&id)
            && let Some(background) = &node.hover_background
        {
            element.style().background = Some(background.clone());
        }
        if self.publication.surface.node(id).and_then(|node| node.props.get("metadata"))
            .is_some_and(|m| m["wordEllipsis"] == true)
        {
            let label: SharedString = self.shown_text(id, 2).into();
            let t = &self.table.tokens;
            let mut text = window.text_style();
            text.font_family = pm_design::TEXT_FONT.into();
            text.refine(&node.style.text);
            let font_size = text.font_size.to_pixels(window.rem_size());
            let leading = font_size * t.typography.reading_leading;
            let line = window.text_system().shape_line(label.clone(), font_size, &[text.to_run(label.len())]);
            // Leave room for the layout engine to snap the measured width to pixels.
            let width = px(f32::from(line.width).ceil());
            return Err(element.w(width).h(leading).min_w(px(0.))
                .child(canvas(
                    move |bounds, window, _| {
                        if line.width <= bounds.size.width {
                            return Some(line.clone());
                        }
                        let shape = |s: SharedString| {
                            let run = text.to_run(s.len());
                            window.text_system().shape_line(s, font_size, &[run])
                        };
                        // Remove complete trailing words; keep the full label in the button's AX name.
                        label.rmatch_indices(char::is_whitespace)
                            .map(|(at, _)| format!("{}…", label[..at].trim_end()).into())
                            .chain(std::iter::once(SharedString::from("…")))
                            .map(shape)
                            .find(|line| line.width <= bounds.size.width)
                    },
                    move |bounds, line, window, cx| {
                        if let Some(line) = line {
                            let _ = line.paint(bounds.origin, leading, gpui::TextAlign::Left, None, window, cx);
                        }
                    },
                ).absolute().inset_0())
                .into_any_element());
        }
        if self
            .prepared
            .timeline_time
            .is_some_and(|(_, transport)| transport == id)
            && self.publication.panel == "timeline"
            && self.natives.views.borrow().contains_key("timeline")
        {
            let m = &self.table.tokens.metrics;
            element = element
                .pr(px(m.gap))
                .min_w(px(0.))
                .overflow_hidden();
        }
        if let Some(build) = &self.panel_build {
            if id == 0 || node.parent == Some(0) {
                element.style().background = None;
            }
            if node.component == Some(Component::InspectorRow) {
                let index = if build.explicit {
                    self.build_rows()
                        .iter()
                        .position(|row| *row == id)
                        .unwrap_or(0)
                } else {
                    0
                };
                element = element
                    .filter(crate::generated_panel::dot_filter(
                        Some(build),
                        Some(index),
                        &self.table.tokens,
                    ))
                    .opacity(crate::generated_panel::control_opacity(
                        Some(build),
                        index,
                        self.table.tokens.accessibility.reduce_motion,
                    ));
            } else if node.text.is_some() {
                let mut parent = node.parent;
                let mut in_row = false;
                while let Some(id) = parent {
                    let Some(n) = self.prepared.node(id) else {
                        break;
                    };
                    if n.component == Some(Component::InspectorRow) {
                        in_row = true;
                        break;
                    }
                    parent = n.parent;
                }
                if !in_row {
                    element = element.filter(crate::generated_panel::dot_filter(
                        Some(build),
                        None,
                        &self.table.tokens,
                    ));
                }
            }
        }
        // In a body that scrolls, the root and the panel fill the body by
        // growing into it, not as a share of a height that their content
        // sets (which would leave them their content's height).
        if self.grows && (id == 0 || node.parent == Some(0)) {
            element.style().size.height = None;
            element = element.flex_grow(1.);
        }
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
        if let Some((labels, room)) = &node.fits_labels {
            let mut text = window.text_style();
            text.refine(&node.style.text);
            let size = text.font_size.to_pixels(window.rem_size());
            let widest = labels
                .iter()
                .map(|label| {
                    let run = text.to_run(label.len());
                    window
                        .text_system()
                        .shape_line(label.clone(), size, &[run])
                        .width
                })
                .fold(px(0.), |a, b| a.max(b));
            element = element.min_w(widest + px(*room)).flex_shrink_0();
        }
        if node.outline_column && self.natives.outline.column.get() > 0. {
            element = element.w(px(self.natives.outline.column.get()));
        }
        if node.outline_retained && self.natives.outline.scene.borrow().is_some() {
            // Retained rows are placed at their scene tops below, so their
            // container must stay at the content origin for every range.
            element = element.absolute().left(px(0.)).right(px(0.)).top_0();
        } else if let Some(first) = &node.outline_first {
            element = element
                .absolute()
                .left(px(0.))
                .right(px(0.))
                .top(px(self.natives.outline.top(first)));
        }
        element = self.place_outline_row(node, element);
        #[cfg(test)]
        {
            let name = node
                .semantic
                .as_ref()
                .map_or_else(|| format!("n{id}"), |s| s.to_string());
            element = element.debug_selector(|| name);
        }
        if let Some(scroll) = self.scrolls.get(&id) {
            // A scroller may shrink to nothing unless its panel asks to keep
            // some rows (`metadata.rowFloor`, Effect Controls' effects): a
            // panel body shorter than those rows scrolls instead.
            let floor = self.row_floor(id);
            // A scroll region with a max height (a sheet body) keeps its
            // content height up to that cap; panel scrollers fill their dock.
            element = if node.style.max_size.height.is_some() {
                element.flex_grow(1.).flex_shrink(1.)
            } else if self.grows {
                // In a body that scrolls the panel's height comes from its
                // content: a scroller adds only its floor, then fills.
                element.flex_grow(1.).flex_shrink(1.).flex_basis(px(floor))
            } else {
                element.flex_1()
            };
            element = element.min_h(px(floor)).track_scroll(scroll);
            // `metadata.scrollAnchor: "end"` (a conversation): a scroller that
            // was at its end stays there as content grows; once scrolled up,
            // it stays where the person left it.
            let metadata = self.publication.surface.node(id).and_then(|n| n.props.get("metadata"));
            let anchor = metadata.and_then(|m| m["scrollAnchor"].as_str());
            let follows = anchor == Some("end");
            let starts = anchor == Some("start");
            if follows && scroll.offset().y <= -scroll.max_offset().y + px(1.) {
                scroll.scroll_to_bottom();
            }
            if node.scroll_y {
                element = element.overflow_y_scroll();
            }
            // Project replays wheel steps within a viewport of its layout.
            let fits = scroll.max_offset().y <= px(0.);
            if self.layered() && !self.holds_native(id) && !(fits && self.culls_in(id)) {
                element = element.scroll_layer();
            }
            if follows || starts {
                // A new token or anchor changes where the scroll area starts.
                let token = metadata.map_or(Value::Null, |m| m["scrollToken"].clone());
                let mut follow = self.follow.borrow_mut();
                let f = follow.entry(id).or_insert_with(|| Follow {
                    token: token.clone(),
                    ..Follow::default()
                });
                if f.token != token || f.anchor_start != starts {
                    f.token = token;
                    f.anchor_start = starts;
                    f.away = false;
                    if starts {
                        scroll.set_offset(point(px(0.), px(0.)));
                    } else {
                        scroll.scroll_to_bottom();
                    }
                }
            }
            if let Some(threshold) = metadata.and_then(|m| m["scrollCompactBelow"].as_f64()) {
                let view = cx.entity().downgrade();
                let known = self.follow.borrow().get(&id).and_then(|f| f.compact);
                element = element.child(canvas(
                    move |bounds, _, cx| {
                        let compact = f64::from(f32::from(bounds.size.height)) < threshold;
                        if known != Some(compact) {
                            let view = view.clone();
                            cx.defer(move |cx| {
                                let _ = view.update(cx, |v, cx| {
                                    let previous = v.follow.borrow_mut().entry(id).or_default()
                                        .compact.replace(compact);
                                    if previous != Some(compact) {
                                        v.send(id, "scroll", EventDetail {
                                            value: Some(json!({ "compact": compact })),
                                            ..Default::default()
                                        }, cx);
                                    }
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                ).absolute().inset_0());
            }
            if follows {
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
        if self.publication.panel == "agent" && node.semantic.as_deref() == Some("agent-composer") {
            let measured = self.agent_composer_height.clone();
            element = element.child(canvas(
                move |bounds, window, cx| {
                    let height = f32::from(bounds.size.height);
                    if (measured.replace(height) - height).abs() > 0.5 {
                        let window = window.window_handle();
                        cx.defer(move |cx| {
                            let _ = cx.update_window(window, |_, window, _| window.refresh());
                        });
                    }
                },
                |_, _, _, _| {},
            ).absolute().inset_0());
        }
        if let Some(geometry) = node.virtual_window {
            element = element.on_scroll_wheel(cx.listener(move |_, _, window, cx| {
                cx.defer_in(window, move |v, _, cx| v.range_change(id, geometry, cx));
            }));
            // A list laid out from its width (a grid's columns) hears of a
            // new width as soon as a frame lays it out (measured after
            // layout: a panel maximized or resized draws nothing more until
            // something else does, so a check while rendering saw the size
            // before and the grid kept its old columns). A new height asks
            // for the rows that show now: a list built while its slot was a
            // row tall kept a page of one row as the slot grew (PM-387).
            let known = self.list_widths.get(&id).copied();
            let view = cx.entity().downgrade();
            element = element.child(
                canvas(
                    move |bounds, _, cx| {
                        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                        if size.0 > 0. && size.1 > 0. && known != Some(size) {
                            let view = view.clone();
                            cx.defer(move |cx| {
                                let _ = view.update(cx, |v, cx| {
                                    if v.list_widths.get(&id) != Some(&size) {
                                        v.list_widths.insert(id, size);
                                        v.range_change(id, geometry, cx);
                                    }
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        }
        if let Some(close) = self
            .publication
            .surface
            .node(id)
            .and_then(|n| n.props.get("metadata"))
            .and_then(|m| m.get("compositionTabClose"))
        {
            let active = close["active"].as_bool().unwrap_or(false);
            let shown = close["visible"].as_bool().unwrap_or(active);
            let mut values = self.motion.values.borrow_mut();
            let value = values
                .entry(id)
                .or_insert_with(|| pm_design::motion::Animated::unit(if active { 1. } else { 0. }));
            let target = if shown { 1. } else { 0. };
            if value.target[0] != target {
                value.to(
                    target,
                    self.table.tokens.spring(if shown {
                        pm_design::motion::Preset::HoverIn
                    } else {
                        pm_design::motion::Preset::HoverOut
                    }),
                );
            }
            let amount = value.get().clamp(0., 1.);
            if value.moving() {
                window.request_animation_frame();
            }
            element = element.opacity(amount);
        }
        // A floating surface springs in when it appears and out while its
        // panel keeps it for the exit (`pmKit.closing`): opacity, and a
        // short rise from `motion.start_scale` worth of its height.
        if let Some(amount) = self.presence.get(&id).copied()
            && amount < 1.
        {
            let t = &self.table.tokens;
            let rise = if node.resource.is_some() {
                0.
            } else {
                (1. - amount) * (1. - t.motion.start_scale as f32) * 160.
            };
            element = element.opacity(amount).mt(px(rise));
        }
        element = self.accessible(element, node);
        if let Some(focus) = self.focus.get(&id) {
            // No focus ring (owner decision, 2026-11-14): buttons and
            // toggles show none, before or after a click.
            // A tracked handle carries its own tab settings (the element's
            // are ignored): `tabIndex` -1 stays focusable but out of the Tab
            // order, as on the web (a segmented control's other segments).
            let tab = node.tab_index.unwrap_or(0);
            let focus = &focus.clone().tab_stop(tab >= 0).tab_index(tab.max(0));
            element = element.track_focus(focus).tab_index(tab.max(0));
        }
        if self.quote_owner(id) == Some(id) {
            if self.keyboard_focused_user_message == Some(id) {
                let (ink, width, _) = self.table.tokens.focus_ring();
                element = element.focus_visible_style(move |style| {
                    style.border(px(width)).border_color(rgb(ink)).rounded(px(width))
                });
            }
            let reveal = self
                .publication
                .surface
                .node(id)
                .and_then(|n| n.props.get("metadata"))
                .and_then(|m| m.get("revealQuote"))
                .and_then(Value::as_array);
            let mut selected = self.quote_selection.borrow_mut();
            if let Some(range) = reveal {
                let nonce = range.get(2).and_then(Value::as_u64).unwrap_or(0);
                if selected.reveal_seen != Some((id, nonce)) {
                    selected.reveal_seen = Some((id, nonce));
                    let mut blocks = Vec::new();
                    self.quote_blocks(id, &mut blocks);
                    let text = blocks
                        .into_iter()
                        .map(|(_, text)| text)
                        .collect::<Vec<_>>()
                        .join("\n");
                    let byte = |offset: usize| {
                        text.char_indices()
                            .scan(0, |units, (i, c)| {
                                let current = *units;
                                *units += c.len_utf16();
                                Some((i, current))
                            })
                            .find(|(_, units)| *units >= offset)
                            .map_or(text.len(), |(i, _)| i)
                    };
                    selected.owner = Some(id);
                    selected.revealed = Some(id);
                    selected.anchor =
                        byte(range.first().and_then(Value::as_u64).unwrap_or(0) as usize);
                    selected.head =
                        byte(range.get(1).and_then(Value::as_u64).unwrap_or(0) as usize);
                    let mut child = id;
                    while let Some(parent) = self.prepared.node(child).and_then(|n| n.parent) {
                        if let Some(scroll) = self.scrolls.get(&parent) {
                            if let Some(index) = self
                                .prepared
                                .node(parent)
                                .and_then(|p| p.children.iter().position(|i| *i == child))
                            {
                                scroll.scroll_to_item(index);
                            }
                        }
                        child = parent;
                    }
                }
            } else {
                if selected.revealed == Some(id) {
                    selected.owner = None;
                    selected.revealed = None;
                }
                if selected.reveal_seen.is_some_and(|(owner, _)| owner == id) {
                    selected.reveal_seen = None;
                }
            }
            drop(selected);
            element = element
                .cursor_text()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, event: &MouseDownEvent, window, cx| {
                        // A click can change the focus origin without moving focus.
                        if v.keyboard_focused_user_message == Some(id) {
                            v.keyboard_focused_user_message = None;
                            cx.notify();
                        }
                        let at = v.quote_selection.borrow().hit(id, event.position);
                        if let Some(at) = at {
                            if let Some(focus) = v.focus.get(&id) {
                                window.focus(focus, cx);
                            }
                            let mut selected = v.quote_selection.borrow_mut();
                            selected.owner = Some(id);
                            selected.revealed = None;
                            selected.anchor = at;
                            selected.head = at;
                            selected.dragging = true;
                            if event.click_count == 2 {
                                let text = selected.text(id);
                                let word = crate::v4_quote::word_range(&text, at);
                                selected.anchor = word.start;
                                selected.head = word.end;
                                selected.dragging = false;
                            }
                            drop(selected);
                            v.publish_quote(id, cx);
                            cx.notify();
                            cx.stop_propagation();
                        }
                    }),
                )
                .on_mouse_move(cx.listener(move |v, event: &MouseMoveEvent, _, cx| {
                    let mut selected = v.quote_selection.borrow_mut();
                    if selected.dragging && selected.owner == Some(id) {
                        if let Some(at) = selected.hit(id, event.position) {
                            selected.head = at;
                        }
                        drop(selected);
                        v.publish_quote(id, cx);
                        cx.notify();
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |v, event: &MouseUpEvent, _, cx| {
                        let click = {
                            let mut selection = v.quote_selection.borrow_mut();
                            let click = selection.dragging
                                && selection.owner == Some(id)
                                && selection.range().is_empty()
                                && selection.hit(id, event.position) == Some(selection.anchor);
                            selection.dragging = false;
                            click
                        };
                        if click && let Some(href) = v.quote_link(id, event.position) {
                            crate::platform::open_url(&href);
                        }
                        cx.notify();
                    }),
                )
                .on_mouse_down_out(cx.listener(move |v, _, _, cx| {
                    if v.quote_selection.borrow().owner == Some(id) {
                        {
                            let mut selected = v.quote_selection.borrow_mut();
                            selected.head = selected.anchor;
                            selected.dragging = false;
                        }
                        v.publish_quote(id, cx);
                        cx.notify();
                    }
                }))
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(move |v, _, _, cx| {
                        v.quote_selection.borrow_mut().dragging = false;
                        cx.notify();
                    }),
                );
        }
        element = self.interactive(element, node, cx);
        element = self.decorate(element, node, window, cx);
        if let Some(drawing) = self.grade_canvas(id, &node.kit, node.disabled) {
            element = element.relative().child(drawing);
            if let Some(h) = self.grade_height(id, &node.kit) {
                element = element.h(px(h));
            }
            // A thumbnail is a drawing: its press goes to what holds it.
            let thumbnail = matches!(
                node.kit,
                Kit::Ease {
                    thumbnail: true,
                    ..
                }
            );
            if !node.disabled && !thumbnail {
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
        let offscreen_control = self.offscreen_control(node, window, cx);
        let has_offscreen_control = offscreen_control.is_some();
        if has_offscreen_control {
            element = if node.component == Some(Component::TextField) {
                element.flex_1().min_w(px(0.))
            } else {
                element.justify_center().items_end()
            };
        }
        let mut children: Vec<AnyElement> = offscreen_control.into_iter().collect();
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
        if let Some(text) = self.disabled_field_text(node).filter(|_| !has_offscreen_control) {
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
        let ring_only = self.publication.surface.node(id).and_then(|n| n.props.get("metadata"))
            .and_then(|m| m.get("selectionRingOnly")) == Some(&Value::Bool(true));
        // A cell can keep its id when list switches to grid. Its old hover
        // animation must not paint a wash behind a ring-only grid card.
        let wash = if ring_only { 0. } else { self.motion.hovers.amount(id) };
        let pressed = self.pressed == Some(id) && !ring_only;
        if (wash > 0. || pressed) && !node.disabled {
            let t = &self.table.tokens;
            let alpha = if pressed {
                (t.translucency.selected * 255.).round() as u32
            } else {
                (t.translucency.hover * wash * 255.).round() as u32
            };
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
        Ok((Box::new(element), children, shown))
    }
    /// A node's element with its children: the edges, the probe, the sheet
    /// and its place in the panel.
    #[inline(never)]
    fn element_close(
        &self,
        id: Id,
        element: Box<Stateful<Div>>,
        mut children: Vec<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(node) = self.prepared.node(id) else {
            return element.into_any_element();
        };
        if let Some(geometry) = node.virtual_window {
            let (_, after) = geometry.spacers();
            children.push(div().flex_none().h(px(after as f32)).into_any_element());
        }
        children.extend(self.cull.record(id));
        if let Some(surface) = &node.native
            && let Some(view) = self.natives.views.borrow().get(surface.as_ref())
        {
            children.push(
                view.clone()
                    .cached(StyleRefinement::default().size_full())
                    .into_any_element(),
            );
        }
        let mut element = (*element).children(children).children(self.probe(node.id));
        // Over the content, last: where it runs past the scroller's edges.
        if let Some(ground) = node.scroll_edge.filter(|_| node.scroll_y)
            && let Some(scroll) = self.scrolls.get(&id)
        {
            element = element.child(scroll_edges(scroll.clone(), ground, &self.table.tokens));
        }
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
        let placed = self.place(element.into_any_element(), node, window, cx);
        if node.outline_first.is_some() {
            return self.outline_layer(id, placed);
        }
        placed
    }
    /// The timeline outline's rows in a layer the surface scrolls: all
    /// rows' height of content at the surface's offset, JS's window of rows
    /// placed at its first row's top.
    /// Each row of the outline stands at its own row's top in the timeline's
    /// scene, not after the row before it: the rows' publication lands later
    /// than the bars (an undo, a redo, a split), and stacked in its own order
    /// it put every name below the change beside the next layer's bar
    /// (PM-376). A row the scene no longer has is not drawn. Its own frame,
    /// out of the recursive render's.
    #[inline(never)]
    fn place_outline_row(&self, node: &NodeView, element: Stateful<Div>) -> Stateful<Div> {
        let Some(parent) = node.parent.and_then(|p| self.prepared.node(p)) else {
            return element;
        };
        if !parent.outline_retained && parent.outline_first.is_none() {
            return element;
        }
        let Some(row) = node.semantic.as_ref().and_then(|s| {
            s.strip_prefix("timeline:row:")
                .or_else(|| s.strip_prefix("timeline:heading:"))
        }) else {
            return element;
        };
        let outline = &self.natives.outline;
        if outline.waiting_for_composition.get() {
            return element.absolute().top_0().invisible();
        }
        // No timeline drawing the bars: the rows' own order stands.
        if outline.scene.borrow().is_none() {
            return element;
        }
        match outline.row_top(row) {
            Some(top) => element
                .absolute()
                .left(px(0.))
                .right(px(0.))
                .top(px(top - if parent.outline_retained { 0. } else {
                    outline.top(parent.outline_first.as_ref().unwrap())
                })),
            None => element.absolute().top(px(0.)).invisible(),
        }
    }
    fn outline_layer(&self, id: Id, rows: AnyElement) -> AnyElement {
        let outline = &self.natives.outline;
        outline
            .handle
            .set_offset(point(px(0.), px(-outline.scroll_y.get())));
        div()
            .id(("v4-outline-rows", id))
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .overflow_y_scroll()
            .track_scroll(&outline.handle)
            .scroll_layer_driven()
            .child(div().w_full().h(px(outline.content_height())))
            .child(rows)
            .into_any_element()
    }
    /// A sheet shown in its own window (Export) is the whole window.
    fn own_sheet(&self, node: &NodeView, window: &Window) -> bool {
        node.component == Some(Component::SheetAnchor)
            && self.publication.panel == "export"
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
        self.field_value_or_placeholder(node)
    }

    fn field_value_or_placeholder(&self, node: &NodeView) -> Option<String> {
        let props = &self.publication.surface.node(node.id)?.props;
        let text = props
            .get("value")
            .filter(|v| !v.is_null())
            .map(|value| match (node.number, value.as_f64()) {
                (Some(number), Some(n)) => number.text(n),
                _ => value_text(value),
            })
            .unwrap_or_default();
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
        let mut wrapper = div().id(("v4-popup", id)).occlude().child(element);
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
            let view = cx.entity().downgrade();
            let peers = self.other_menu_triggers(anchor.unwrap_or(id)).into_iter()
                .map(|r| Bounds::new(point(px(r.x), px(r.y)), size(px(r.w), px(r.h))))
                .collect();
            wrapper = pm_shell::kit::dismiss_menu_on_press(
                wrapper,
                anchor.and_then(|id| self.anchor_bounds(id)),
                peers,
                move |_, cx| {
                    let _ = view.update(cx, |v, cx| {
                        v.send(
                            id,
                            "keydown",
                            EventDetail {
                                key: Some("Escape".into()),
                                ..Default::default()
                            },
                            cx,
                        );
                    });
                },
            );
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
        let opens_field = node.opens_field;
        if !disabled
            && self.publication.panel == "agent"
            && node.semantic.as_deref() == Some("agent-send")
        {
            element = element.on_mouse_down(
                MouseButton::Left,
                cx.listener(|v, _: &MouseDownEvent, window, cx| {
                    if v.prepared
                        .semantics
                        .get("agent-composer-text")
                        .and_then(|id| v.fields.get(id))
                        .is_some_and(|field| field.read(cx).has_keyboard())
                    {
                        window.prevent_default();
                    }
                }),
            );
        }
        if !disabled && (clickable || events.double_click) {
            let node_layer_drag = node.timeline_layer_drag.is_some();
            element = element.on_click(cx.listener(move |v, event: &ClickEvent, window, cx| {
                // Releasing a reorder back over its header must not also
                // select/rename it. Esc keeps this suppressed until the next press.
                if node_layer_drag
                    && matches!(event, ClickEvent::Mouse(_))
                    && v.natives.outline.layer_dragged.get()
                {
                    cx.stop_propagation();
                    return;
                }
                // A click that opens a field (a layer's rename, the typed
                // time): ⌘A, digits and Return typed from now on are the
                // field's, even before it has the keyboard (pm_shell::typing).
                if opens_field.is_some_and(|clicks| event.click_count() >= clicks) {
                    pm_shell::typing::expect();
                }
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
            let effect_target = node
                .semantic
                .as_ref()
                .is_some_and(|s| s.starts_with("layer-effects:effect:"));
            element = element
                .on_drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(target))
                .on_drop(cx.listener(move |v, paths: &ExternalPaths, window, cx| {
                    cx.stop_propagation();
                    let detail = file_drop(paths.paths(), window.modifiers().alt);
                    let received = gpui_macos::ReceivedDrop::for_paths(paths.paths());
                    v.send_with_drop(id, "drop", detail, received, cx);
                }))
                .on_drag_move(
                    cx.listener(move |v, event: &DragMoveEvent<PanelDrag>, _, cx| {
                        if effect_target
                            && event.drag(cx).panel == v.publication.panel
                            && event.bounds.contains(&event.event.position)
                        {
                            v.anchors.borrow_mut().insert(id, event.bounds);
                            let y = f64::from(
                                f32::from(event.event.position.y - event.bounds.origin.y)
                                    / f32::from(event.bounds.size.height).max(1.),
                            );
                            v.send(
                                id,
                                "dragover",
                                EventDetail {
                                    y: Some(y),
                                    ..Default::default()
                                },
                                cx,
                            );
                            // The browser's usual edge scrolling, for the native stack.
                            for scroll in v.scrolls.values() {
                                let bounds = scroll.bounds();
                                if bounds.contains(&event.event.position) {
                                    let edge = v.table.tokens.metrics.row;
                                    let direction = if event.event.position.y
                                        < bounds.top() + px(edge)
                                    {
                                        1.
                                    } else if event.event.position.y > bounds.bottom() - px(edge) {
                                        -1.
                                    } else {
                                        0.
                                    };
                                    if direction != 0. {
                                        let offset = scroll.offset();
                                        let y = (offset.y + px(direction * edge / 2.))
                                            .clamp(-scroll.max_offset().y, px(0.));
                                        scroll.set_offset(point(offset.x, y));
                                        cx.notify();
                                    }
                                }
                            }
                        }
                    }),
                )
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
                    let y = if effect_target {
                        v.anchor_bounds(id).map(|bounds| {
                            f64::from(
                                f32::from(window.mouse_position().y - bounds.origin.y)
                                    / f32::from(bounds.size.height).max(1.),
                            )
                        })
                    } else {
                        None
                    };
                    let detail = EventDetail {
                        y,
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
                label: self.publication.surface.node(id)
                    .and_then(|surface| surface.props.get("metadata"))
                    .and_then(|metadata| metadata["dragLabel"].as_str())
                    .map(Into::into)
                    .or_else(|| node.ax.as_ref().and_then(|ax| ax.label.clone()))
                    .unwrap_or_else(|| "Media".into()),
                item: node.drag_item.clone(),
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
        // A button inside a layer header owns its press as well as its
        // click. Stopping only the click is too late: the row would already
        // have armed a reorder while the button was held.
        let row_control = clickable && node.timeline_layer_drag.is_none() && {
            let mut parent = node.parent;
            let mut in_layer = false;
            while let Some(id) = parent {
                let Some(ancestor) = self.prepared.node(id) else {
                    break;
                };
                if ancestor.timeline_layer_drag.is_some() {
                    in_layer = true;
                    break;
                }
                parent = ancestor.parent;
            }
            in_layer
        };
        if row_control && !events.pointer() && node.timeline_parent_pick.is_none() {
            element = element.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, _: &MouseDownEvent, window, cx| {
                    v.focus_pressed(id, window, cx);
                    cx.stop_propagation();
                }),
            );
        }
        if node.number.is_some() {
            element = element
                .on_scroll_wheel(
                    cx.listener(move |v, event, window, cx| v.wheel_number(id, event, window, cx)),
                )
                .cursor_col_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, event: &MouseDownEvent, window, cx| {
                        v.scrub_press(id, event, window, cx);
                        cx.stop_propagation();
                    }),
                );
        }
        if let Some(layer) = node.timeline_parent_pick.clone() {
            let outline = self.natives.outline.clone();
            element =
                element.on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                    outline.parent_pick(&layer, event.position, cx);
                    cx.stop_propagation();
                });
        }
        if let Some((layer, path)) = node.timeline_scrub.clone().filter(|_| node.number.is_none()) {
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
        if events.pointer() || node.timeline_layer_drag.is_some() {
            element = element.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |v, event: &MouseDownEvent, window, cx| {
                    // Stopping here also stops GPUI's own focus transfer:
                    // the press gives the keyboard to the item it lands in.
                    if !window.default_prevented() {
                        v.focus_pressed(id, window, cx);
                    }
                    v.press(id, event, cx);
                    if events.pointer() {
                        cx.stop_propagation();
                    }
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
        let ring_only = props.and_then(|p| p.get("metadata")).and_then(|m| m.get("selectionRingOnly")) == Some(&Value::Bool(true));
        let wash = clickable && (node.component.is_some() || control_role) && !ring_only;
        if wash {
            // Pressed while the button is down on it; any release ends it.
            element = element
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |v, _: &MouseDownEvent, _, cx| {
                        v.pressed = Some(id);
                        cx.notify();
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|v, _: &MouseUpEvent, _, cx| {
                        if v.pressed.take().is_some() {
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|v, _: &MouseUpEvent, _, cx| {
                        if v.pressed.take().is_some() {
                            cx.notify();
                        }
                    }),
                );
        }
        let group = node.hover_group;
        if wash || events.hover || group {
            element = element.on_hover(cx.listener(move |v, hovered: &bool, _, cx| {
                // While the panel scrolls, the rows move under a still
                // pointer: its hover moves without a draw or a wash, and the
                // draw that ends the scroll shows it (PM-732: each hover
                // crossed drew Properties again, and its wash every frame).
                if v.scrolling() {
                    if wash {
                        v.motion.hovers.set(id, *hovered);
                    }
                } else {
                    // The pointer crossed a group: draw again, which finds
                    // the groups under it (`hovered_groups`).
                    if group {
                        let _ = hovered;
                        cx.notify();
                    }
                    if wash {
                        let tokens = v.table.tokens.clone();
                        v.motion.hovers.hover(id, *hovered, &tokens);
                        cx.notify();
                    }
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
            let (source, id) = (cx.entity().downgrade(), node.id);
            element = element.tooltip(move |_, cx| {
                let (title, tokens, source) = (title.clone(), tokens.clone(), source.clone());
                cx.new(|_| Tooltip::following(source, id, title, tokens))
                    .into()
            });
        }
        element
    }
    /// The node holding keyboard focus, including inline action transitions.
    pub(crate) fn focused_node(&self, window: &Window) -> Option<Id> {
        self.focus
            .iter()
            .find(|(_, h)| h.is_focused(window))
            .map(|(id, _)| *id)
    }
    /// The native shortcut queue runs before GPUI keydown. Forward Delete
    /// to the effect list when this panel owns keyboard focus.
    pub(crate) fn delete_focused_effect(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(focused) = self.focused_node(window) else {
            return false;
        };
        let Some(&list) = self.prepared.semantics.get("layer-effects:list") else {
            return false;
        };
        if self.fields.contains_key(&focused) {
            return false;
        }
        let selected = self.publication.surface.node(list).is_some_and(|node| {
            node.props
                .get("metadata")
                .and_then(|m| m.get("focus"))
                .and_then(Value::as_str)
                .is_some()
        });
        if focused != list && !selected {
            return false;
        }
        self.send(
            list,
            "keydown",
            EventDetail {
                key: Some("delete".into()),
                ..Default::default()
            },
            cx,
        );
        true
    }

    /// A key its window's root took while nothing in the panel was focused
    /// (a sheet just opened): the panel's root hears it as its own keys,
    /// so Return presses a sheet's default button and Escape cancels it.
    pub(crate) fn root_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let root = self
            .publication
            .surface
            .node(0)
            .and_then(|n| n.children.first().copied());
        if let Some(root) = root
            && self.prepared.has_keydown
        {
            self.key_down(root, event, window, cx);
            return true;
        }
        false
    }
    fn key_down(
        &mut self,
        root: Id,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let copy_quote = event.keystroke.modifiers.platform && event.keystroke.key == "c";
        let cite_quote = event.keystroke.modifiers.platform
            && event.keystroke.modifiers.shift
            && matches!(event.keystroke.key.as_str(), "." | ">");
        if cite_quote && self.cite_selection(window, cx) {
            cx.stop_propagation();
            return;
        }
        if copy_quote && self.copy_selection(window, cx) {
            cx.stop_propagation();
            return;
        }
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
        if key == "Escape" {
            let mut at = Some(id);
            while let Some(node) = at.and_then(|id| self.prepared.node(id)) {
                if let Kit::Anchored { anchor: Some(trigger), dismiss: true, .. } = node.kit {
                    self.focus_pressed(trigger, window, cx);
                    break;
                }
                at = node.parent;
            }
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
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = &self.table.tokens;
        if self.publication.surface.node(node.id).and_then(|n| n.props.get("metadata"))
            .and_then(|m| m.get("tooltipWhenTruncated")) == Some(&Value::Bool(true)) {
            let text: SharedString = self.shown_text(node.id, 3).into();
            let mut style = window.text_style();
            // The surface's root font is applied after its children are built.
            style.font_family = pm_design::TEXT_FONT.into();
            style.font_size = px(t.typography.body).into();
            style.refine(&node.style.text);
            let size = style.font_size.to_pixels(window.rem_size());
            let run = style.to_run(text.len());
            let width = window.text_system().shape_line(text, size, &[run]).width;
            let truncated = self.truncated.clone();
            let id = node.id;
            let view = cx.entity().downgrade();
            element = element.child(canvas(move |bounds, _, cx| {
                let clipped = width > bounds.size.width;
                if truncated.borrow_mut().insert(id, clipped) != Some(clipped) {
                    cx.defer(move |cx| { let _ = view.update(cx, |_, cx| cx.notify()); });
                }
            }, |_, _, _, _| {}).absolute().inset_0());
        }
        if self.prepared.anchors.contains(&node.id)
            || self.quote_owner(node.id) == Some(node.id)
            || self.publication.surface.node(node.id).and_then(|node| node.props.get("metadata"))
                .is_some_and(|m| m["menuAnchor"] == true)
        {
            let (id, anchors) = (node.id, self.anchors.clone());
            let wake = self
                .prepared
                .timeline_time
                .filter(|&(time, transport)| id == time || id == transport)
                .and_then(|_| self.installed.clone());
            element = element.child(
                canvas(
                    move |bounds, _, _| {
                        let changed = anchors.borrow_mut().insert(id, bounds) != Some(bounds);
                        if changed && let Some(wake) = &wake {
                            let _ = wake.try_send(());
                        }
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
                    .rounded(px((t.metrics.inner_radius - t.space.xs).max(t.space.hair)))
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
                        .animate_when_visible()
                        .into_any_element()
                });
            }
            Kit::QueueProgress { value, ring } => {
                if *ring {
                    let value = *value;
                    let accent = t.colors.accent;
                    let track = t.colors.field;
                    let stroke = t.metrics.export_job_progress_height;
                    let side = t.metrics.symbol_panel;
                    element = element.child(canvas(|_,_,_|(), move |bounds,_,window,_| {
                        let center = bounds.center();
                        let radius = side / 2. - stroke;
                        for (fraction, color) in [(1., track), (value, accent)] {
                            let mut path = PathBuilder::stroke(px(stroke));
                            for step in 0..=48 {
                                let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * fraction * step as f32 / 48.;
                                let p = center + point(px(radius * angle.cos()), px(radius * angle.sin()));
                                if step == 0 { path.move_to(p); } else { path.line_to(p); }
                            }
                            if let Ok(path) = path.build() { window.paint_path(path, rgb(color)); }
                        }
                    }).size(px(side)).flex_none());
                } else {
                    element = element.child(div().h(px(t.metrics.export_job_progress_height)).w_full()
                        .rounded(px(t.metrics.border)).overflow_hidden().bg(rgb(t.colors.field))
                        .child(div().h_full().w(relative(*value)).bg(rgb(t.colors.accent))));
                }
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
            Kit::Wheel { .. }
            | Kit::Curve { .. }
            | Kit::Ease { .. }
            | Kit::Pad { .. }
            | Kit::Ramp { .. } => {}
            Kit::Disclosure { open, always_show } => {
                let open = self
                    .disclosures
                    .get(&node.id)
                    .copied()
                    .unwrap_or(*open);
                let group: SharedString = format!("disclosure-{}", node.id).into();
                element = element.group(group.clone()).child(
                    div()
                        .when(open && !always_show, |d| {
                            d.opacity(0.).group_hover_style(group, |s| s.opacity(1.))
                        })
                        .child(pm_shell::symbol::symbol(
                            if open {
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
        // A symbol leading a line of text stands in the text's leading
        // inset, level with its first line; the words keep the width and
        // wrap under themselves (a row would keep them on one line).
        if node.symbol_leads {
            element = element
                .relative()
                .pl(px(node.symbol_size + t.metrics.gap / 2.));
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
            let glyph = pm_shell::symbol::symbol(
                symbol,
                node.symbol_size,
                pm_shell::symbol::Weight::Regular,
                ink,
                window,
            );
            element = if node.symbol_leads {
                element.child(div().absolute().left_0().top_0().child(glyph))
            } else {
                element.child(glyph)
            };
        }
        // A picture the host made (a Project thumbnail): it fills the node,
        // cropped to its shape; until it exists the node shows its fill.
        if let Some(path) = node
            .resource
            .as_ref()
            .and_then(|key| crate::media_thumbs::resolve(key))
        {
            let mut picture = img(path).absolute().inset_0().size_full().object_fit(
                if node.contain {
                    ObjectFit::Contain
                } else {
                    ObjectFit::Cover
                },
            );
            picture.style().corner_radii = node.style.corner_radii.clone();
            element = element.child(picture);
        }
        if let Some(text) = &node.text {
            if node.component == Some(Component::PopUpButton) {
                // Native slots return before decoration. This is the kit's
                // real non-AppKit rendering, using the same published title.
                element = element
                    .gap(px(t.metrics.gap))
                    .child(div().flex_1().min_w(px(0.)).truncate().child(text.clone()))
                    .child(pm_shell::symbol::symbol(
                        "chevron.up.chevron.down",
                        t.metrics.symbol_inline,
                        pm_shell::symbol::Weight::Regular,
                        if node.disabled { t.colors.muted } else { t.colors.secondary },
                        window,
                    ));
            } else {
                element = element.child(self.plain_or_quote_text(node.id, text));
            }
        }
        if let Some(rich) = &node.rich {
            let text = rich_text(rich, t, self.quote_range(node.id));
            if self.quote_owner(node.id).is_some() || rich.links.is_empty() {
                element = element.child(self.quote_text(node.id, text, rich.text.to_string()));
            } else {
                let links = rich.links.clone();
                element = element.child(
                    InteractiveText::new(("prose-link", node.id), text).on_click(
                        links.iter().map(|(range, _)| range.clone()).collect(),
                        move |index, _, _| {
                            if let Some((_, href)) = links.get(index) {
                                crate::platform::open_url(href);
                            }
                        },
                    ),
                );
            }
        }
        if let Some(number) = node.number.filter(|_| self.draws_number(node)) {
            let shown = self.scrub_readout(node.id).unwrap_or(number.value);
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
                .on_scroll_wheel(
                    cx.listener(move |v, event, window, cx| v.wheel_number(id, event, window, cx)),
                )
                .cursor_col_resize()
                .capture_any_mouse_down(cx.listener(
                    move |v, event: &MouseDownEvent, window, cx| {
                        if event.button == MouseButton::Left {
                            v.scrub_press(id, event, window, cx);
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
    /// What is dragged, when the row says (an Effects row: its effect id).
    pub item: Option<SharedString>,
    /// Floating fill, its edge, text.
    colors: (u32, u32, u32),
}
#[cfg(test)]
impl PanelDrag {
    /// A drag from `panel`'s row named `label`, carrying `item`.
    pub(crate) fn of(panel: &str, label: &str, item: Option<&str>) -> Self {
        Self {
            panel: panel.into(),
            label: label.to_owned().into(),
            item: item.map(|i| i.to_owned().into()),
            colors: (0, 0, 0),
        }
    }
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
    /// A virtual list's visible rows and width (`rangechange`).
    fn range_change(
        &mut self,
        id: Id,
        geometry: pm_builtins::virtualization::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scroll) = self.scrolls.get(&id) else {
            return;
        };
        let bounds = scroll.bounds();
        let height = f32::from(bounds.size.height) as f64;
        // A wheel step can leave the offset past the end until the next
        // frame clamps it: read it within the list, or the range is empty
        // and the list shows no rows.
        let end = (geometry.count as f64 * geometry.item_size - height).max(0.);
        let offset = (-f32::from(scroll.offset().y) as f64).clamp(0., end);
        if let Ok(range) = geometry.visible(offset, height) {
            let width = f32::from(bounds.size.width);
            let sent = RangeSent {
                range,
                width: width.to_bits(),
                count: geometry.count,
                published: geometry.range,
            };
            self.request_range(
                id,
                RangeRequest::List(sent),
                EventDetail {
                    range: Some(range),
                    width: Some(width as f64),
                    ..Default::default()
                },
                cx,
            );
        }
    }
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
        let painted = std::mem::take(&mut *self.painted_groups.borrow_mut());
        self.hovered_groups = painted
            .into_iter()
            .filter(|(_, hitbox)| hitbox.is_hovered(window))
            .map(|(id, _)| id)
            .collect();
        if self.focus_requested.is_some() && pm_shell::typing::expecting() {
            self.typing_ready(cx);
        }
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
        self.plan_cull(window, cx);
        let viewport = window.viewport_size();
        let t = &self.table.tokens;
        let root = div().relative().w_full();
        let root = if self.grows {
            // Measured each frame: a change resizes the body's content.
            let natural = self.natural.clone();
            let column_natural = self.column_natural.clone();
            let scrolls: Vec<(Id, ScrollHandle)> =
                self.scrolls.iter().map(|(id, h)| (*id, h.clone())).collect();
            // The vertical scrollers shown, with their floors: each fills what
            // the panel leaves it (`element`), so the panel's height is
            // measured without what they stretched past their floor.
            let fillers: Vec<(Id, ScrollHandle, f32)> = self
                .prepared
                .nodes
                .values()
                .filter(|n| n.scroll_y && !n.hidden && !self.left_out(n.id))
                .filter_map(|n| {
                    Some((n.id, self.scrolls.get(&n.id)?.clone(), self.row_floor(n.id)))
                })
                .collect();
            let origin = self.origin.clone();
            let entity = cx.entity().downgrade();
            let (panel, fit) = (self.publication.panel.clone(), self.fit.get());
            root.min_h(px(self.fit.get()))
                .on_children_prepainted(move |children, window, _| {
                    // Measured as painted, the panel would keep whatever height
                    // it was given once (a scroller fills it, and is measured
                    // filling it): its content then never grows past that old
                    // height, nor shrinks back. Without the stretch it is the
                    // content's own height; the scroller scrolls the rest.
                    let painted = children.first().map_or(0., |b| f32::from(b.size.height));
                    let stretch = fillers.iter().fold(0f32, |most, (_, scroll, floor)| {
                        most.max(f32::from(scroll.bounds().size.height) - floor)
                    });
                    let height = (painted - stretch).max(0.);
                    // What the panel asks of its dock: that height with each
                    // scroller's content in full, measured from where its
                    // children end (their layout, which a viewport stretched
                    // or squeezed does not move), so a block added or folded
                    // changes it both ways. Lists of their own (a floor) keep
                    // what they ask.
                    let pages = fillers.iter().fold(0f32, |most, (_, scroll, floor)| {
                        most.max(content_height(scroll) - floor)
                    });
                    let demand = scrolls.iter().fold(height + pages, |height, (id, scroll)| {
                        if fillers.iter().all(|(page, ..)| page != id) {
                            height.max(f32::from(
                                scroll.bounds().bottom() - origin.get().y + scroll.max_offset().y,
                            ))
                        } else {
                            height
                        }
                    });
                    let changed = (natural.replace(height) - height).abs() > 0.5;
                    let column_changed = (column_natural.replace(demand) - demand).abs() > 0.5;
                    if (changed || column_changed) && crate::diag::enabled() {
                        crate::diag::line(serde_json::json!({"ui": "natural",
                            "panel": panel.as_str(), "painted": painted, "stretch": stretch,
                            "natural": height, "demand": demand, "fit": fit}));
                    }
                    // Prepaint is still drawing: the panel draws again on
                    // the next frame (and with it the shell, which reads the
                    // new height). Only this panel: refreshing the window drew
                    // every panel again, cached or not, on every wheel step
                    // that measured a row (PM-696). A change of the demand
                    // alone (rows measured as a scroll brings them near)
                    // waits until the steps stop.
                    if changed {
                        let entity = entity.clone();
                        window.on_next_frame(move |_, cx| {
                            let _ = entity.update(cx, |_, cx| cx.notify());
                        });
                    } else if column_changed {
                        let entity = entity.clone();
                        window.on_next_frame(move |_, cx| {
                            let _ = entity.update(cx, |v, cx| v.settle_demand(cx));
                        });
                    }
                })
        } else {
            root.h_full()
        };
        root.flex()
            .flex_col()
            .text_color(rgb(t.colors.text))
            .text_size(px(t.typography.body))
            .font_family(pm_design::TEXT_FONT)
            .child(self.element(0, window, cx, self.prepared.header))
            .child(Self::tracking(cx, self.origin.clone()))
            .children(self.colour.as_ref().map(|(id, picker, _)| {
                let bounds = self.anchors.borrow().get(id).copied().unwrap_or_default();
                let width=px(t.metrics.color_picker_width);
                let height=px(t.metrics.color_picker_plane_height+t.metrics.control+2.*t.space.md+t.space.sm);
                let margin=px(t.space.lg);let gap=px(t.space.sm);
                let x=(bounds.origin.x+bounds.size.width-width).max(margin).min((viewport.width-width-margin).max(margin));
                let below=bounds.origin.y+bounds.size.height+gap;
                let y=if below+height<=viewport.height-margin {below}else{(bounds.origin.y-gap-height).max(margin)};
                deferred(
                    anchored()
                        .position(point(x,y))
                        .snap_to_window()
                        .child(picker.clone()),
                )
                .priority(3)
            }))
            .children(self.cull.outer_mark())
            .children(self.body_rest_on_move(cx))
            .children(self.body_wheel_watch(cx))
    }
}
pub(crate) struct Tooltip {
    title: SharedString,
    tokens: pm_design::Tokens,
    /// The node it describes: its title is read again at each draw, so a
    /// press that changes it (a disclosure's Expand to Collapse, applied when
    /// the panel republishes) never leaves the old words up while hovered.
    source: Option<(WeakEntity<SurfaceView>, Id)>,
}
impl Tooltip {
    /// A tooltip for node `id` of `view`, showing `title` until it changes.
    pub(crate) fn following(
        view: WeakEntity<SurfaceView>,
        id: Id,
        title: SharedString,
        tokens: pm_design::Tokens,
    ) -> Self {
        Self {
            title,
            tokens,
            source: Some((view, id)),
        }
    }
    /// Its node's title now (the last one, when the node has none left).
    pub(crate) fn current_title(&mut self, cx: &App) -> SharedString {
        if let Some(title) = self.source.as_ref().and_then(|(view, id)| {
            view.upgrade()
                .and_then(|view| view.read(cx).tooltip_title(*id))
        }) {
            self.title = title;
        }
        self.title.clone()
    }
}
impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.current_title(cx);
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
fn rich_text(
    rich: &crate::v4_compile::Rich,
    t: &pm_design::Tokens,
    selected: Option<std::ops::Range<usize>>,
) -> StyledText {
    use crate::v4_compile::Rich;
    let ink = |c: u32| gpui::rgb_to_hsla(rgb(c));
    let highlights = rich
        .runs
        .iter()
        .map(|(range, bits)| {
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
            let mut boundaries = vec![range.start, range.end];
            if let Some(selected) = &selected {
                for i in [selected.start, selected.end] {
                    if i > range.start && i < range.end {
                        boundaries.push(i);
                    }
                }
            }
            boundaries.sort_unstable();
            boundaries
                .windows(2)
                .map(|pair| {
                    let mut style = style.clone();
                    if selected.as_ref().is_some_and(|s| s.contains(&pair[0])) {
                        style.background_color = Some(ink(t.colors.selected_fill));
                        style.color = Some(ink(t.colors.selected_text));
                    }
                    (pair[0]..pair[1], style)
                })
                .collect::<Vec<_>>()
        })
        .flatten();
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
        options.push(json!({
            "label": child.props.get("label"),
            "symbol": child.props.get("attr").and_then(|a| a.get("symbol")),
            "tip": child.props.get("title"),
            "disabled": child.props.get("disabled"),
        }));
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
                if let (Some(value), Some(o)) = (
                    row.and_then(|m| m.get(key)).cloned(),
                    option.as_object_mut(),
                ) {
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
    vec![
        json!({"min":n("min").unwrap_or(0.),"max":n("max").unwrap_or(1.),"step":n("step").unwrap_or(0.),"log":log}),
    ]
}
fn control_kind(panel: &str, node: &pm_surface::Node) -> Option<crate::v4_control::Kind> {
    use crate::v4_control::Kind;
    match Component::of(node) {
        Some(Component::PopUpButton)
            if node.props.get("metadata").and_then(|m| m.get("manualMenu"))
                == Some(&Value::Bool(true)) =>
        {
            Some(Kind::PopupTrigger)
        }
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
        // A colour well its panel marks native (Settings): AppKit's own.
        Some(Component::ColourWell)
            if node.props.get("metadata").and_then(|m| m.get("native"))
                == Some(&Value::Bool(true)) =>
        {
            Some(Kind::Well)
        }
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
        // Every answer below is the node's own title or none: most nodes
        // have none, and the words they show (a walk and a join) are asked
        // of every control on every render.
        node.title.as_ref()?;
        let shown = self.shown_text(id, 3);
        let props = self.publication.surface.node(id).map(|n| &n.props);
        if props.is_some_and(|p| p.get("metadata").is_some_and(|m| m["menuAnchor"] == true)) {
            return props
                .filter(|p| p.get("aria-expanded").and_then(Value::as_bool) != Some(true))
                .and_then(|_| node.title.clone());
        }
        let menu_value = props.is_some_and(|p| p.contains_key("aria-haspopup"))
            && (!shown.trim().is_empty()
                || props.is_some_and(|p| p.get("value").is_some_and(|v| !v.is_null())));
        if props.and_then(|p| p.get("metadata")).and_then(|m| m.get("tooltipWhenTruncated")) == Some(&Value::Bool(true)) {
            return self.truncated.borrow().get(&id).copied().unwrap_or(false).then(|| node.title.clone()).flatten();
        }
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
        let header = self.view.update(cx, |v, cx| match v.prepared.header {
            Some(id) => {
                let skip = if tabbed { v.header_title(id) } else { None };
                v.element(id, window, cx, skip)
            }
            None => div().into_any_element(),
        });
        // A cached view lays its element out as a root of its own, which a
        // root with no width sizes to its content: the header then never had
        // its row's width, and groups meant to share the row (the viewer
        // bar's leading and trailing ones) shrank to less than what cannot
        // give way in them, so the 2D | 3D control stood over the title and
        // the pop-ups (PM-463). Alone in its row, the header takes the row.
        if tabbed {
            header
        } else {
            div().size_full().child(header).into_any_element()
        }
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

    #[gpui::test]
    fn queue_busy_range_is_marked_sent_only_after_ack(cx: &mut gpui::TestAppContext) {
        use crate::v4_view_tests::{rig, settle, window};
        use pm_surface::events::EventDetail;
        use std::time::{Duration, Instant};

        let rig = rig("timeline", 2);
        rig.runtime.take_publications();
        let panel = "synthetic-range";
        let graph = json!({
            "sources": {
                "powermove:bundle": "export * from './index.tsx';",
                "index.tsx": "import { jsx } from 'powermove/jsx-runtime';
                    export function activate({commands}) {
                        commands.register({id:'hold', title:'Hold'}, () =>
                            new Promise(resolve => setTimeout(resolve, 1000)));
                        commands.register({id:'fill', title:'Fill'}, () => {});
                    }
                    export function view() {
                        return jsx('div', {id:'synthetic-list', onRangeChange: () => {}});
                    }"
            },
            "edges": {
                "powermove:bundle": {"./index.tsx":"index.tsx"},
                "index.tsx": {"powermove/jsx-runtime":"runtime:powermove/jsx-runtime"}
            }
        });
        let mut mount = rig.runtime.mount_extension(panel, &graph, None).unwrap();
        pm_runtime_v8::wait_until(&mut mount, Instant::now() + Duration::from_secs(20))
            .unwrap()
            .unwrap();
        let first = rig
            .runtime
            .take_publications()
            .into_iter()
            .find(|publication| publication.panel == panel)
            .expect("fixture panel published");
        let id = first
            .surface
            .nodes()
            .values()
            .find(|node| node.props.get("id") == Some(&json!("synthetic-list")))
            .unwrap()
            .id;
        let (view, mut test_window) = window(cx, &rig, first, Default::default());
        let _hold = rig.runtime.run_command(panel, "hold", &[]).unwrap();
        for _ in 0..=pm_runtime_v8::Limits::default().queue_capacity {
            rig.runtime.run_command(panel, "fill", &[]).unwrap();
        }
        assert!(
            rig.runtime
                .script_queue_stats(panel)
                .unwrap()
                .deferred_discrete
                > 0
        );
        let rows = (5, 15);
        let range = super::RangeRequest::Outline {
            rows,
            published: (0, 5),
        };
        view.update(&mut test_window, |v, cx| {
            v.request_range(
                id,
                range,
                EventDetail {
                    range: Some(rows),
                    ..Default::default()
                },
                cx,
            );
            assert!(!v.sent_ranges.contains_key(&id));
            assert!(v.pending_ranges.contains_key(&id));
        });
        settle(cx, |cx| {
            view.read_with(cx, |v, _| v.sent_ranges.get(&id) == Some(&range))
        });
    }

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
    /// The shared picker uses the same keyframe-aware binding as the Properties row.
    #[gpui::test]
    fn colour_picker_live_preview_commits_one_undo_and_keys_animated_color(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::v4_view_tests::{PROJECT, project_json, publication, rig_project, settle, window};
        use pm_surface::kit::Component;
        use serde_json::Value;
        use std::time::Duration;
        let project=project_json(1).replace("\"w\":100,\"h\":100", "\"w\":100,\"h\":100,\"color\":{\"v\":\"#FF0000\",\"kf\":[{\"t\":0,\"v\":\"#FF0000\"},{\"t\":2,\"v\":\"#0000FF\"}]}");
        let rig = rig_project("inspector", &project);
        rig.kernel
            .call(
                PROJECT,
                pm_kernel::Request::new(
                    pm_kernel::Caller::native(),
                    pm_kernel::SelectionCall::Layers {
                        ids: vec!["l0".into()],
                        add: false,
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        rig.runtime.update_facts(json!({"time":1.,"playing":false}));
        for (_, ticket) in rig.runtime.publish() {
            let ticket = ticket.unwrap();
            while ticket.try_take().is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let (view, mut vcx) = window(cx, &rig, publication(&rig), Default::default());
        vcx.run_until_parked();
        let before = rig.kernel.published(PROJECT).unwrap().history.entries;
        let original = rig
            .kernel
            .published(PROJECT)
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .to_value();
        let picker = vcx.update(|window, cx| {
            view.update(cx, |v, cx| {
                let id = v
                    .publication
                    .surface
                    .nodes()
                    .values()
                    .find(|n| Component::of(n) == Some(Component::ColourWell))
                    .expect("Properties colour field")
                    .id;
                v.open_colour(id, window, cx);
                v.colour.as_ref().unwrap().1.clone()
            })
        });
        picker.update(&mut vcx, |p, cx| {
            p.input_numeric(6, "127.5", cx);
            p.input_numeric(7, "63.75", cx);
            p.input_numeric(8, "191.25", cx);
        });
        settle(&mut vcx, |cx| {
            view.read_with(cx, |v, _| v.scrub_gesture.is_some() && !v.scrub_busy && v.scrub_ops.is_empty())
        });
        assert_eq!(
            rig.kernel.published(PROJECT).unwrap().history.entries,
            before,
            "live preview has no separate undo entries"
        );
        picker.update(&mut vcx, |p, cx| p.finish(true, cx));
        settle(&mut vcx, |cx| {
            view.read_with(cx, |v, _| v.colour.is_none() && !v.scrub_busy && v.scrub_ops.is_empty())
        });
        let published = rig.kernel.published(PROJECT).unwrap();
        assert_eq!(published.history.entries, before + 1);
        let root = published.snapshot.root().unwrap();
        let location = pm_kernel::model::find_layer(root, "l0").unwrap();
        let layer = pm_kernel::model::layer_at(root, &location).unwrap().to_value();
        let json: Value = serde_json::from_str(&pm_model::json::to_canonical_string(&layer)).unwrap();
        let keys = json["d"]["color"]["kf"]
            .as_array()
            .expect("animated color");
        assert!(
            keys.iter()
                .any(|k| k["t"].as_f64() == Some(1.) && k["v"] == json!("#8040BF")),
            "{keys:?}"
        );
        rig.kernel
            .call(
                PROJECT,
                pm_kernel::Request::new(
                    pm_kernel::Caller::native(),
                    pm_kernel::Call::History(pm_kernel::HistoryCall::Undo),
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
        assert_eq!(
            rig.kernel
                .published(PROJECT)
                .unwrap()
                .snapshot
                .root()
                .unwrap()
                .to_value(),
            original
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

impl SurfaceView {
    /// The height a scroller keeps however short its panel's body
    /// (`metadata.rowFloor` rows; Effect Controls' effects); 0 for one that
    /// may shrink to nothing.
    fn row_floor(&self, id: Id) -> f32 {
        let Some(node) = self.prepared.node(id) else {
            return 0.;
        };
        self.publication
            .surface
            .node(id)
            .and_then(|n| n.props.get("metadata"))
            .and_then(|m| m.get("rowFloor"))
            .and_then(Value::as_f64)
            .filter(|rows| rows.is_finite() && *rows > 0.)
            .map_or(0., |rows| {
                // Rows of a virtual list are its own height.
                let row = node
                    .virtual_window
                    .map_or(self.table.tokens.metrics.row, |w| w.item_size as f32);
                rows as f32 * row
            })
    }
    /// Natural content demand includes inner lists before they are clipped.
    /// The panel's demand changed while measuring rows: it draws again (and
    /// the shell reads the demand) once the changes have stopped for a
    /// scroll layer's settle.
    fn settle_demand(&mut self, cx: &mut Context<Self>) {
        self.demand_changes += 1;
        let at = self.demand_changes;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEMAND_SETTLE).await;
            let _ = this.update(cx, |v, cx| {
                if v.demand_changes == at {
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(crate) fn natural_body_height(&self) -> f32 {
        self.natural.get().max(self.column_natural.get())
    }

    pub(crate) fn agent_panel_minimum_height(&self) -> f32 {
        let tokens = &self.table.tokens;
        let minimum = tokens.agent_panel_minimum_height();
        if self.prepared.semantics.contains_key("agent-composer") {
            minimum.max(self.agent_composer_height.get() + tokens.metrics.header + tokens.metrics.text_padding)
        } else {
            minimum
        }
    }

    /// The panel fills its body and grows with longer content.
    pub(crate) fn in_body(&self, body: f32) -> gpui::StyleRefinement {
        gpui::StyleRefinement::default()
            .w_full()
            .h(px(self.body_height(body)))
    }
    /// The panel's height in a body that scrolls: its content, at least
    /// the body's slot.
    pub(crate) fn body_height(&self, body: f32) -> f32 {
        self.fit.set(body);
        body.max(self.natural.get())
    }
}

impl Drop for SurfaceView {
    fn drop(&mut self) {
        for id in self.fields.keys() {
            pm_mcp::coedit::touch_field(
                self.runtime.project_id(),
                &format!("{}/{}", self.publication.panel, id),
                false,
            );
        }
    }
}
