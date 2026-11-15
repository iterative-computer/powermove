//! Native timeline host element. Pointer input runs Rust gesture sessions against
//! the kernel on a worker; JS keeps the outline and hears about it by publication.
//! Layer-header reorders share the native pointer gesture lifecycle.
//! No sample waits on V8, and the main thread never waits on the kernel.
use crate::v4_view::Outline;
use gpui::{prelude::*, *};
use pm_design::Tokens;
use pm_kernel::{Caller, KernelHandle, Published, QueryMirror, SelectionCall};
use pm_panel_timeline::{
    Context as MenuContext, DragKind, DragSample, EditTool, GraphDisplay, HostEffect,
    MarqueeSession, Modifiers as Mods, Plan, PropertyFacts, RowId, Scrub, TimelineClient,
    TimelineMode, TimelineModel,
    footer::Part,
    frame,
    graph_render::{self, Graph},
    render::{Edge, Frame, Interpolation, Label, Media, ModelPresenter, Op, Rect, Scene, Viewport},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

/// Host services a gesture needs. Implemented by the Controller; tests record.
/// Media dropped on the timeline: placed at `at`, joining the stack at
/// `index` (0 the top; `None` the bottom).
#[derive(Clone, Debug, PartialEq)]
pub struct MediaDrop {
    pub at: f64,
    pub index: Option<usize>,
    pub what: Dropped,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Dropped {
    /// From Finder; Option held keeps a folder's files apart.
    Files {
        paths: Vec<std::path::PathBuf>,
        alt: bool,
    },
    /// What the Project panel is dragging.
    ProjectItems,
}
/// The Effects panel's id (`js/v4/app/panels.json`): its rows drag effects
/// onto layers.
pub const EFFECTS_PANEL: &str = "fxbrowser";

#[cfg(test)]
mod panel_id_tests {
    /// The Effects panel the drop targets listen for is a built-in panel.
    #[test]
    fn the_effects_panel_id_is_a_built_in_panel() {
        assert!(pm_builtins::builtin_panel(super::EFFECTS_PANEL).is_some());
        assert!(pm_builtins::builtin_panel(pm_builtins::PROJECT_PANEL).is_some());
    }
}
/// An effect dragged from the Effects panel `panel` dropped on `layer`'s row
/// (`None`: no row there, and the selected layer takes it, as v1.2.0).
#[derive(Clone, Debug, PartialEq)]
pub struct EffectDrop {
    pub panel: String,
    pub definition: String,
    pub label: String,
    pub layer: Option<String>,
}
pub trait Delegate: 'static {
    fn timing_preview(&self, _snapshot: Option<pm_kernel::Snapshot>, _cx: &mut App) {}
    fn time(&self, cx: &App) -> f64;
    /// Media dropped on the timeline (Finder files, Project items).
    fn drop_media(&self, _drop: MediaDrop, _cx: &mut App) {}
    /// An effect from the Effects panel dropped on the timeline.
    fn drop_effect(&self, _drop: EffectDrop, _cx: &mut App) {}
    fn seek(&self, seconds: f64, cx: &mut App);
    fn effect(&self, effect: HostEffect, cx: &mut App);
    fn report(&self, message: String, cx: &mut App);
    /// Run a shell command by name, as its menu item or chord would.
    fn command(&self, name: &str, cx: &mut App);
    /// A command with arguments (a menu row that names its target).
    fn run(&self, name: &str, _args: Vec<serde_json::Value>, cx: &mut App) {
        self.command(name, cx);
    }
    /// The layer menu's rows for the layer under the pointer
    /// (`layer_commands::layer_menu`).
    fn layer_menu_rows(&self, _layer: &str, _cx: &App) -> Vec<serde_json::Value> {
        vec![]
    }
    /// `choose` runs with the picked row, on the main thread, once tracking ends.
    fn menu(
        &self,
        items: Vec<serde_json::Value>,
        window: &mut Window,
        cx: &mut App,
        choose: Choose,
    );
    /// Asks extension clip painters (`timeline.paint`) for decorations of
    /// `clips`; `done` gets `{clipId: shapes}` merged, on the main thread.
    fn paint_clips(&self, _clips: serde_json::Value, _done: PaintDone, _cx: &mut App) {}
    /// What v1–3's `timeline` service reports moved: view, mode and the
    /// visible rows' layers in window points.
    fn view_facts(&self, _facts: serde_json::Value, _cx: &mut App) {}
    /// Moves when a clip painter arrives or leaves.
    fn painters_revision(&self, _cx: &App) -> u64 {
        0
    }
    /// The time under the panel moved (scroll, zoom, a resize): the native
    /// playhead, drawn outside GPUI, is placed again.
    fn view_moved(&self, _cx: &mut App) {}
    /// The skimmer moved to `seconds` (`None`: it left the timeline).
    fn skim(&self, _seconds: Option<f64>, _cx: &mut App) {}
    /// Whether skimming is on (View ▸ Skimming).
    fn skimming(&self, _cx: &App) -> bool {
        false
    }
    /// Whether the viewer still shows the skimmer's frame.
    fn skimmer_shown(&self, _cx: &App) -> bool {
        true
    }
    /// A context menu whose native rows come first, then what extensions add
    /// for each of `locations` (v1–3's `MenuLocation` and what it was opened on).
    fn extension_menu(
        &self,
        _locations: Vec<(&'static str, serde_json::Value)>,
        items: Vec<serde_json::Value>,
        window: &mut Window,
        cx: &mut App,
        choose: Choose,
    ) {
        self.menu(items, window, cx, choose);
    }
}
/// Receives extension clip decorations, and whether every painter answered.
pub type PaintDone = Box<dyn FnOnce(pm_panel_timeline::render::Decorations, bool, &mut App)>;
/// Runs with the picked row index once a native menu closes.
pub type Choose = Box<dyn FnOnce(Option<usize>, &mut App)>;
#[derive(Clone, Debug)]
enum Spec {
    Layers,
    /// Option-drag: copies follow the pointer, the originals stay.
    Duplicate,
    Trim(Edge),
    Offset,
    Ripple(Edge),
    Roll(Edge),
    Slip,
    Slide,
    Keys,
    GraphKeys,
    /// A grip of the selected keys' transform box (`TimelineSurface::transform`):
    /// the composition time and value it scales about.
    Transform {
        anchor_time: f64,
        anchor_value: f64,
    },
    /// A key's direction handle in the Value Graph.
    Handle {
        key: String,
        side: pm_kernel::commands::keyframes::EaseSide,
    },
    Property {
        layer: String,
        path: String,
    },
    WorkIn,
    WorkOut,
    WorkMove,
    /// ⌥ on a key: stretch the selected keys' timing (the grabbed key's
    /// composition time).
    StretchKeys {
        grabbed: f64,
    },
    QuickOffsetKeys,
    TextStagger {
        band: pm_panel_timeline::stagger::Band,
        end: bool,
    },
}
/// A transform box drag (v1's `dragGraphTransform`): the selected keys'
/// span when the grip was pressed, which grip, and whether ⌘ scales about
/// the box's centre.
#[derive(Clone, Copy, Debug)]
struct TransformBox {
    t0: f64,
    t1: f64,
    v0: f64,
    v1: f64,
    grip: usize,
    centred: bool,
}
impl TransformBox {
    fn sides(&self) -> (bool, bool, bool, bool) {
        let name = graph_render::GRIPS[self.grip];
        (
            name.contains('e'),
            name.contains('w'),
            name.contains('n'),
            name.contains('s'),
        )
    }
    /// The time and value the drag scales about (the opposite edge, or the
    /// centre with ⌘).
    fn anchor(&self) -> (f64, f64) {
        let (east, west, north, south) = self.sides();
        let (mid_t, mid_v) = ((self.t0 + self.t1) / 2., (self.v0 + self.v1) / 2.);
        let time = if self.centred || !(east || west) {
            mid_t
        } else if east {
            self.t0
        } else {
            self.t1
        };
        let value = if self.centred || !(north || south) {
            mid_v
        } else if north {
            self.v0
        } else {
            self.v1
        };
        (time, value)
    }
    /// The scales for a pointer that has travelled `dt` seconds and `dv`
    /// in value from the press: the grabbed edge follows the pointer from
    /// where it was (the grip stands off the keys, so the keys never jump
    /// to the pointer at the first move).
    fn scales(&self, dt: f64, dv: f64) -> (f64, f64) {
        let (east, west, north, south) = self.sides();
        let (at, av) = self.anchor();
        let moving_t = if east { self.t1 } else { self.t0 };
        let moving_v = if north { self.v1 } else { self.v0 };
        let (time, value) = (moving_t + dt, moving_v + dv);
        let ts = if (east || west) && (moving_t - at).abs() > 1e-9 {
            (time - at) / (moving_t - at)
        } else {
            1.
        };
        let vs = if (north || south) && (moving_v - av).abs() > 1e-9 {
            (value - av) / (moving_v - av)
        } else {
            1.
        };
        (ts, vs)
    }
}
enum Job {
    Sync,
    SelectLayer(String, Mods),
    SelectKey {
        layer: String,
        channel: String,
        key: String,
        additive: bool,
        dragging: bool,
    },
    Begin(Spec, f64),
    Sample(DragSample),
    Finish(bool),
    Apply(Plan),
    Command {
        id: String,
        time: f64,
        frames: Option<i32>,
        shift: bool,
    },
    PasteKeys(Vec<pm_panel_timeline::ClipboardTrack>, f64),
    Menu {
        context: MenuContext,
        action: String,
        time: f64,
    },
    Persist(Viewed),
    Reveal {
        layer: String,
        keys: Vec<String>,
    },
    ShowGraph {
        layer: String,
        channel: String,
        keys: Vec<String>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Viewed {
    pps: f64,
    scroll_time: f64,
    scroll_y: f64,
    layer_column: f64,
    /// The person set the column (a drag); only then is it saved with the
    /// project, so a default never sticks as if it had been chosen.
    column_chosen: bool,
}
struct Built {
    scene: Arc<Scene>,
    graph: Option<Arc<Graph>>,
    model: Arc<TimelineModel>,
    /// Audio clips whose waveforms the scene drew as still reading.
    wanted: Vec<crate::waveforms::Key>,
}
/// Audio originals by asset id, for waveforms.
type Sources = Arc<BTreeMap<String, PathBuf>>;
struct Press {
    origin: Point<Pixels>,
    spec: Option<Spec>,
    /// A plain click on an already selected item narrows selection on release.
    release: Option<Job>,
}
enum Gesture {
    ParentPick {
        ids: Vec<String>,
        origin: Point<Pixels>,
        at: Point<Pixels>,
        target: Option<String>,
        moved: bool,
    },
    Idle,
    Pressed(Press),
    Dragging {
        origin: Point<Pixels>,
        proedit: bool,
        readout: bool,
        quick: Option<(Point<Pixels>, f64, usize)>,
        quick_limits: Vec<(f64, f64)>,
        /// A handle drag: its key's layer start, for layer-local handle times.
        handle: Option<f64>,
    },
    Scrub(Scrub),
    Marquee {
        session: MarqueeSession,
        origin: Point<Pixels>,
        at: Point<Pixels>,
    },
    Pan {
        at: Point<Pixels>,
    },
    Column,
    /// The bottom bar: the zoom slider held; the navigator's thumb or the
    /// row thumb held this far from its start.
    Zoom,
    Navigate {
        grab: f64,
    },
    Rows {
        grab: f64,
    },
    /// A layer header pressed in the layer list: a click stays the row's
    /// (it selects on release); past 4 pt it lifts into a reorder.
    LayerPress {
        layer: String,
        origin: Point<Pixels>,
    },
    /// v1's layer reorder (timeline.ts gutterDown): the layers moving and
    /// where they would land.
    Reorder {
        moving: Vec<String>,
        at: Point<Pixels>,
        drop: Option<LayerDrop>,
    },
}
/// Where a reorder would land (v1's `layerDrop`), in panel space.
#[derive(Clone, Debug)]
struct LayerDrop {
    target: String,
    mode: pm_panel_timeline::DropMode,
    /// The insertion line (before/after) and its left end (the depth cue).
    line: f64,
    left: f64,
    /// The target label for the insertion cue.
    name: String,
}
pub struct TimelineSurface {
    kernel: KernelHandle,
    project: String,
    tokens: Tokens,
    delegate: Rc<dyn Delegate>,
    outline: Rc<Outline>,
    client: Option<TimelineClient>,
    queue: VecDeque<Job>,
    busy: bool,
    sample_scheduled: bool,
    viewer_previewing: bool,
    /// Assets this editor's viewer cannot read: their clips show Media
    /// Offline (docs/native/design/media-offline.md).
    offline: Arc<BTreeSet<String>>,
    sources: Sources,
    /// Waveform files being read for this timeline.
    reading: BTreeSet<crate::waveforms::Key>,
    model: Arc<TimelineModel>,
    scene: Option<Arc<Scene>>,
    /// The graph on screen; its value range may be held (`steady_graph`).
    graph: Option<Arc<Graph>>,
    /// The latest graph over its own fitted range, for Fit Selected Curves.
    graph_fitted: Option<Arc<Graph>>,
    /// The value range the wheel panned the graph to, for one view (v1's
    /// `graphViewBounds`).
    graph_view: Option<(String, f64, f64)>,
    /// A transform box drag in progress.
    transform: Option<TransformBox>,
    /// A key drag in the graph: ⇧ also locks it to its main axis (v1).
    graph_axis: bool,
    /// The drag readout's text and where the pointer is (surface points).
    readout: Option<(String, f64, f64)>,
    frame: Option<Arc<Frame>>,
    /// The clip under the pointer and the pointer's x in the panel: its strip
    /// draws hovered, with the trim grip inside the nearer edge.
    hover_clip: Option<(String, f64)>,
    bounds: Bounds<Pixels>,
    /// The panel's whole body: `bounds` (the ruler and the rows) and the
    /// bottom bar under them.
    body: Bounds<Pixels>,
    /// The part of the bottom bar or the row thumb under the pointer or held.
    lit: Option<Part>,
    view: Viewed,
    /// The native playhead's time mapping: pixels per second, first second and
    /// time column of the frame last prepared.
    shown: (f64, f64, f64),
    fitted: bool,
    /// The composition length last shown: when it changes while the view
    /// shows all of it from 0, the view refits to the new length.
    fit_duration: f64,
    gesture: Gesture,
    cursor: CursorStyle,
    /// A badge's tooltip shown after `motion.tooltip_delay` (text, local point).
    tip: Option<(SharedString, (f64, f64))>,
    tip_wait: Option<Task<()>>,
    row_scroll: Option<Task<()>>,
    /// A wheel or pinch view change waiting for the gesture to rest.
    idle_persist: Option<Task<()>>,
    /// Extension clip decorations by clip id (`timeline.paint`), and what
    /// they were asked for (painter revision, visible clips) and the request
    /// in flight; `late` is when a request some painter missed was asked.
    decorations: Arc<pm_panel_timeline::render::Decorations>,
    painted: Option<(u64, String)>,
    painting: bool,
    late: Option<std::time::Instant>,
    /// The service facts last told to the delegate.
    told: Option<serde_json::Value>,
    /// The skimmer's time while the pointer hovers the time area (Final
    /// Cut's skimming): a ghost playhead; the playhead itself stays.
    skimmer: Option<f64>,
    /// Clears the skimmer when the window stops being active (Cmd-Tab, key
    /// loss, hide): the pointer may never move out of the timeline.
    activation: Option<Subscription>,
    /// Where media being dragged over the timeline would land (window
    /// position), for the insertion line.
    media_over: Option<Point<Pixels>>,
    /// An effect from the Effects panel over the timeline: the pointer.
    effect_over: Option<Point<Pixels>>,
}
const WHEEL_ZOOM: f64 = 0.01;
/// The fewest letters of a clip's name worth showing before its ellipsis.
const NAME_LEAST: usize = 3;
const PPS: (f64, f64) = (4., 4000.);
/// How far past the timeline's end the view may run, in points.
const END_ROOM: f64 = 48.;
/// The room a fitted view leaves after the composition's end (v1
/// `frameView`), so the last strips' ends stand clear of the row scroller.
const FIT_ROOM: f64 = 40.;
impl TimelineSurface {
    pub fn new(
        kernel: KernelHandle,
        project: String,
        tokens: Tokens,
        delegate: Rc<dyn Delegate>,
        outline: Rc<Outline>,
        offline: BTreeSet<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let layer_column = default_layer_column(&tokens.metrics);
        let view = Viewed {
            pps: 100.,
            scroll_time: 0.,
            scroll_y: 0.,
            layer_column,
            column_chosen: false,
        };
        outline.column.set(layer_column as f32);
        outline.bind_surface(cx.entity().downgrade());
        let mut this = Self {
            kernel: kernel.clone(),
            project: project.clone(),
            tokens: tokens.clone(),
            delegate,
            outline,
            client: None,
            queue: VecDeque::new(),
            busy: true,
            sample_scheduled: false,
            viewer_previewing: false,
            offline: Arc::new(offline),
            sources: Default::default(),
            reading: Default::default(),
            model: Default::default(),
            scene: None,
            graph: None,
            graph_fitted: None,
            graph_view: None,
            transform: None,
            graph_axis: false,
            readout: None,
            frame: None,
            bounds: Bounds::default(),
            body: Bounds::default(),
            lit: None,
            view,
            shown: (view.pps, view.scroll_time, view.layer_column),
            fitted: false,
            fit_duration: 0.,
            gesture: Gesture::Idle,
            cursor: CursorStyle::Arrow,
            tip: None,
            tip_wait: None,
            row_scroll: None,
            idle_persist: None,
            decorations: Default::default(),
            painted: None,
            painting: false,
            told: None,
            skimmer: None,
            hover_clip: None,
            activation: None,
            media_over: None,
            effect_over: None,
            late: None,
        };
        cx.on_release(|s, cx| {
            if s.viewer_previewing {
                let delegate=s.delegate.clone();
                cx.defer(move |cx|delegate.timing_preview(None,cx));
            }
        })
        .detach();
        this.restore_view();
        // The render bar follows the disk render cache (Render Selection).
        let changes = crate::render_frames::watch();
        cx.spawn(async move |this, cx| {
            while changes.recv().await.is_ok() {
                let alive = this.update(cx, |s, cx| {
                    let Some(scene) = &s.scene else { return };
                    let bars = crate::render_frames::bars(s.model.index.comp_id.as_deref());
                    let mut next = Scene::clone(scene);
                    next.bars = bars;
                    let next = Arc::new(next);
                    s.scene = Some(next.clone());
                    s.outline.set_scene(Some(next));
                    s.frame = None;
                    s.changed(false, cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        let offline = this.offline.clone();
        let sources = this.sources.clone();
        cx.spawn(async move |this, cx| {
            // Indexing the document is O(layers): never on the main thread.
            let created = cx
                .background_executor()
                .spawn(async move {
                    TimelineClient::new(kernel, project, Caller::native()).map(|mut client| {
                        let _ = client.adopt_view();
                        let built = build(&client, &tokens, offline, sources);
                        (client, built)
                    })
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.busy = false;
                match created {
                    Ok((client, built)) => {
                        s.client = Some(client);
                        s.install(built, cx);
                        s.pump(cx);
                    }
                    Err(error) => s.delegate.report(error.to_string(), cx),
                }
            });
        })
        .detach();
        this
    }
    fn restore_view(&mut self) {
        let Some(p) = self.kernel.published(&self.project) else {
            return;
        };
        let ui = &p.ui_state.timeline;
        let number = |key: &str| ui.get(key).and_then(pm_model::Value::as_f64);
        if let Some(pps) = number("nativePps").filter(|p| p.is_finite()) {
            self.view.pps = pps.clamp(PPS.0, PPS.1);
            self.fitted = true;
        }
        if let Some(time) = number("nativeScrollTime").filter(|t| t.is_finite()) {
            self.view.scroll_time = time;
        }
        if let Some(y) = number("nativeScrollY").filter(|y| y.is_finite()) {
            self.view.scroll_y = y.max(0.);
        }
        if let Some(column) = number("nativeLayerColumn").filter(|c| c.is_finite()) {
            // A width saved narrower than a row's least (before it had one)
            // opens at the least.
            let column = column.max(least_layer_column(&self.tokens.metrics));
            self.view.layer_column = column;
            self.view.column_chosen = true;
            self.outline.column.set(column as f32);
        }
    }
    /// A kernel publication landed: follow it on a worker.
    pub fn published(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.queue.back(), Some(Job::Sync)) {
            self.queue.push_back(Job::Sync);
        }
        self.pump(cx);
    }
    pub fn set_offline_media(&mut self, offline: BTreeSet<String>, cx: &mut Context<Self>) {
        if *self.offline != offline {
            self.offline = Arc::new(offline);
            self.published(cx);
        }
    }
    /// Readable audio originals by asset id: their clips draw waveforms.
    pub fn set_audio_sources(
        &mut self,
        sources: BTreeMap<String, PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if *self.sources != sources {
            self.sources = Arc::new(sources);
            self.published(cx);
        }
    }
    pub fn set_tokens(&mut self, tokens: Tokens, cx: &mut Context<Self>) {
        if self.tokens != tokens {
            // A layer column the person never dragged follows the theme's
            // (workspace timeline chrome); a dragged one stays.
            let (old, new) = (
                default_layer_column(&self.tokens.metrics),
                default_layer_column(&tokens.metrics),
            );
            let follows = (self.view.layer_column - old).abs() < 0.5 && old != new;
            self.tokens = tokens;
            if follows {
                self.view.layer_column = new;
                self.changed(false, cx);
            }
            self.frame = None;
            self.published(cx);
        }
    }
    /// Keymap and menu entry: the same model command the panel used to reach.
    pub fn command(
        &mut self,
        id: &str,
        time: f64,
        frames: Option<i32>,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        // The time scale is this surface's own view, not the model's: the
        // keys zoom as the bottom bar's ends do.
        use pm_panel_timeline::footer::ZOOM_STEP;
        match id {
            // The playhead is the caller's: this runs inside the editor's own
            // command, where asking it for the time is not allowed.
            "timeline.zoomIn" => return self.zoom_about(self.view.pps * ZOOM_STEP, time, cx),
            "timeline.zoomOut" => return self.zoom_about(self.view.pps / ZOOM_STEP, time, cx),
            "timeline.fit" => return self.fit_time(cx),
            _ => {}
        }
        self.queue.push_back(Job::Command {
            id: id.into(),
            time,
            frames,
            shift,
        });
        self.pump(cx);
    }
    /// Menu checkmarks: the timeline shows tracks, and clips snap in them.
    pub fn toggles(&self) -> (bool, bool) {
        (
            self.model.mode == TimelineMode::Tracks,
            !self.model.track_snap_disabled,
        )
    }
    /// The edit tool a clip drag uses (View menu checkmarks).
    pub fn edit_tool(&self) -> pm_panel_timeline::EditTool {
        self.model.edit_tool
    }
    /// View ▸ Hide Shy Layers is on.
    pub fn hides_shy(&self) -> bool {
        self.model.hide_shy
    }
    /// The keyframe clipboard payload, or nothing with no keys selected.
    pub fn copied_keys(&self) -> Vec<pm_panel_timeline::ClipboardTrack> {
        self.model.copy_keys(self.model.selection.keys())
    }
    /// Pastes each copied track back on its own channel at the playhead.
    pub fn paste_keys(
        &mut self,
        tracks: Vec<pm_panel_timeline::ClipboardTrack>,
        time: f64,
        cx: &mut Context<Self>,
    ) {
        self.queue.push_back(Job::PasteKeys(tracks, time));
        self.pump(cx);
    }
    /// Escape: cancel the active gesture. Foreign edits survive in the kernel.
    pub fn cancel(&mut self, cx: &mut Context<Self>) -> bool {
        self.row_scroll = None;
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Idle => return false,
            Gesture::Dragging { .. } => {
                self.queue.retain(|job| !matches!(job, Job::Sample(_)));
                self.sample_scheduled = false;
                self.queue.push_back(Job::Finish(false));
            }
            Gesture::Marquee { session, .. } => {
                self.queue.push_back(Job::Apply(session.cancel()));
            }
            _ => {}
        }
        cx.notify();
        self.pump(cx);
        true
    }
    pub fn fit_time(&mut self, cx: &mut Context<Self>) {
        let width = (f32::from(self.bounds.size.width) as f64 - self.view.layer_column).max(1.);
        if self.model.index.duration > 0. {
            self.view.pps =
                ((width - FIT_ROOM).max(1.) / self.model.index.duration).clamp(PPS.0, PPS.1);
            self.view.scroll_time = 0.;
            self.changed(true, cx);
        }
    }
    /// View ▸ Merge Selected Curves is on.
    pub fn graph_merged(&self) -> bool {
        self.model.graph_merge
    }
    /// Fit Selected Curves: the span from the graph's first key to its last
    /// fills the time area, with a little room at both ends.
    fn fit_span(&mut self, first: f64, last: f64, cx: &mut Context<Self>) {
        const ROOM: f64 = 24.;
        let width = f32::from(self.bounds.size.width) as f64 - self.view.layer_column - 2. * ROOM;
        let frame = 1. / self.model.index.fps.max(1.);
        self.view.pps = (width.max(1.) / (last - first).max(frame)).clamp(PPS.0, PPS.1);
        self.view.scroll_time = (first - ROOM / self.view.pps).max(self.model.index.minimum_time());
        // The values refit too: the curves fill the graph's height again.
        self.graph_view = None;
        if self.graph_fitted.is_some() {
            self.graph = self.graph_fitted.clone();
        }
        self.fitted = true;
        self.changed(true, cx);
    }
    fn zoom_at(&mut self, x: f64, factor: f64, cx: &mut Context<Self>) {
        let anchor = self.view.scroll_time + (x - self.view.layer_column) / self.view.pps;
        self.view.pps = (self.view.pps * factor).clamp(self.least_pps(), PPS.1);
        self.view.scroll_time = anchor - (x - self.view.layer_column) / self.view.pps;
        self.clamp_time();
        self.fitted = true;
        self.changed_while_hot(cx);
    }
    /// The time area's width in points (none before the first layout).
    fn time_width(&self) -> Option<f64> {
        let width = f32::from(self.bounds.size.width) as f64 - self.view.layer_column;
        (width > END_ROOM).then_some(width)
    }
    /// The farthest a person can zoom out: the whole of the timeline's time
    /// (`minimum_time` to `maximum_time`) fits the time area with a little
    /// room at its end, never more.
    fn least_pps(&self) -> f64 {
        let index = &self.model.index;
        let span = (index.maximum_time() - index.minimum_time()).max(1. / index.fps.max(1.));
        self.time_width()
            .map_or(PPS.0, |w| ((w - END_ROOM) / span).clamp(PPS.0, PPS.1))
    }
    /// Keeps the view on the timeline's time: from its earliest time to a
    /// little past its end, so scrolling or zooming never leaves it on empty
    /// time past the composition.
    fn clamp_time(&mut self) {
        let index = &self.model.index;
        let least = index.minimum_time();
        let most = self.time_width().map_or(f64::INFINITY, |w| {
            index.maximum_time() + END_ROOM / self.view.pps - w / self.view.pps
        });
        self.view.scroll_time = self.view.scroll_time.min(most).max(least);
    }
    /// The ruler and the rows: the body less the bottom bar, which a body
    /// too short for both gives up.
    fn above_bar(body: Bounds<Pixels>, t: &Tokens) -> Bounds<Pixels> {
        let bar = px(t.metrics.ruler);
        if body.size.height < bar * 4. {
            return body;
        }
        Bounds::new(body.origin, size(body.size.width, body.size.height - bar))
    }
    /// The bottom bar's layout for the view as it stands.
    fn bar(&self) -> Option<pm_panel_timeline::footer::Layout> {
        let scene = self.scene.as_ref()?;
        let (top, height) = (
            f32::from(self.bounds.size.height) as f64,
            f32::from(self.body.size.height - self.bounds.size.height) as f64,
        );
        (height > 0.).then_some(pm_panel_timeline::footer::Layout {
            width: f32::from(self.bounds.size.width) as f64,
            top,
            height,
            layer_column: self.view.layer_column,
            pps: self.view.pps,
            pps_range: PPS,
            scroll_time: self.view.scroll_time,
            time: (self.model.index.minimum_time(), scene.duration),
            rows_top: self.tokens.metrics.ruler as f64,
            scroll_y: self.view.scroll_y,
            // Under a graph the rows do not scroll.
            content_height: if self.graph_region().is_some() {
                0.
            } else {
                scene.content_height()
            },
        })
    }
    fn bar_ops(&self) -> Vec<Op> {
        let (Some(bar), Some(scene)) = (self.bar(), self.scene.as_ref()) else {
            return vec![];
        };
        let length = crate::captions_panel::timecode(scene.duration, scene.fps);
        bar.ops(self.lit, &length, &self.tokens)
    }
    /// The bar's part at `(x, y)`. A strip's trim zone beside the row
    /// scroller's track wins over the track: a strip that runs to the view's
    /// end is trimmed there (the thumb itself still wins).
    fn bar_part(&self, x: f64, y: f64) -> Option<pm_panel_timeline::footer::Part> {
        let part = self.bar()?.hit(x, y, &self.tokens)?;
        let trims = self
            .frame
            .as_ref()
            .and_then(|f| f.hit_clip(x, y))
            .is_some_and(|(_, edge)| edge.is_some());
        (!(trims && part == pm_panel_timeline::footer::Part::RowsTrack)).then_some(part)
    }
    /// Lights the part of the bar under an idle pointer; true over the bar
    /// or the row thumb, where the tracks' hover and skimmer stand down.
    fn over_bar(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let (x, y) = self.local(position);
        let inside = self.body.contains(&position);
        let part = self
            .bar()
            .filter(|_| inside)
            .and_then(|_| self.bar_part(x, y));
        let over = part.is_some() || (inside && y >= f32::from(self.bounds.size.height) as f64);
        if over && self.cursor != CursorStyle::Arrow {
            self.cursor = CursorStyle::Arrow;
            cx.notify();
        }
        if self.lit != part {
            self.lit = part;
            cx.notify();
        }
        over
    }
    /// Zooms to `pps` about the playhead when it is on screen, else about
    /// the middle of the tracks.
    fn zoom_to(&mut self, pps: f64, cx: &mut Context<Self>) {
        let time = self.delegate.time(cx);
        self.zoom_about(pps, time, cx);
    }
    /// [`Self::zoom_to`] with the playhead at `time`.
    fn zoom_about(&mut self, pps: f64, time: f64, cx: &mut Context<Self>) {
        let width = f32::from(self.bounds.size.width) as f64;
        let column = self.view.layer_column;
        let playhead = column + (time - self.view.scroll_time) * self.view.pps;
        let about = if playhead >= column && playhead <= width {
            playhead
        } else {
            (column + width) / 2.
        };
        self.zoom_at(about, pps / self.view.pps, cx);
    }
    fn press_bar(
        &mut self,
        part: Part,
        bar: pm_panel_timeline::footer::Layout,
        x: f64,
        y: f64,
        clicks: usize,
        cx: &mut Context<Self>,
    ) {
        use pm_panel_timeline::footer::ZOOM_STEP;
        let t = &self.tokens;
        self.lit = Some(part);
        match part {
            Part::ZoomOut => self.zoom_to(self.view.pps / ZOOM_STEP, cx),
            Part::ZoomIn => self.zoom_to(self.view.pps * ZOOM_STEP, cx),
            // Twice on the slider fits the composition (the first press has
            // brought the knob under the pointer); once on the track sets
            // the zoom there and holds the knob.
            Part::Zoom | Part::ZoomTrack if clicks == 2 => self.fit_time(cx),
            Part::Zoom => self.gesture = Gesture::Zoom,
            Part::ZoomTrack => {
                self.gesture = Gesture::Zoom;
                self.drag_bar(Part::Zoom, x, y, 0., cx);
            }
            Part::Time | Part::TimeTrack => {
                let (from, length) = bar.time_thumb(t).unwrap_or_default();
                let grab = if part == Part::Time {
                    x - from
                } else {
                    length / 2.
                };
                self.gesture = Gesture::Navigate { grab };
                self.drag_bar(Part::Time, x, y, grab, cx);
            }
            Part::Rows | Part::RowsTrack => {
                let (from, length) = bar.rows_thumb().unwrap_or_default();
                let grab = if part == Part::Rows {
                    y - from
                } else {
                    length / 2.
                };
                self.gesture = Gesture::Rows { grab };
                self.drag_bar(Part::Rows, x, y, grab, cx);
            }
        }
        cx.notify();
    }
    fn drag_bar(&mut self, part: Part, x: f64, y: f64, grab: f64, cx: &mut Context<Self>) {
        let Some(bar) = self.bar() else {
            return;
        };
        match part {
            Part::Zoom => {
                if let Some(pps) = bar.zoom_at(x, &self.tokens) {
                    self.zoom_to(pps, cx);
                }
            }
            Part::Time => {
                if let Some(time) = bar.scroll_time_at(x - grab, &self.tokens) {
                    self.view.scroll_time = time;
                    self.clamp_time();
                    self.fitted = true;
                    self.changed_while_hot(cx);
                }
            }
            _ => {
                if let Some(scroll) = bar.scroll_y_at(y - grab) {
                    self.view.scroll_y = scroll;
                    self.clamp_scroll();
                    self.changed_while_hot(cx);
                }
            }
        }
    }
    /// Wheel and pinch move the view locally on every event; each write
    /// publishes to every panel, so it waits until the gesture rests.
    fn changed_while_hot(&mut self, cx: &mut Context<Self>) {
        self.changed(false, cx);
        self.idle_persist = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::gizmo::NAVIGATION_IDLE)
                .await;
            let _ = this.update(cx, |s, cx| {
                s.idle_persist = None;
                s.changed(true, cx);
            });
        }));
    }
    fn changed(&mut self, persist: bool, cx: &mut Context<Self>) {
        self.frame = None;
        self.outline.scrolled(
            self.view.scroll_y as f32,
            self.view.layer_column as f32,
            self.visible_rows(),
            cx,
        );
        if persist {
            self.queue.retain(|j| !matches!(j, Job::Persist(_)));
            self.queue.push_back(Job::Persist(self.view));
            self.pump(cx);
        }
        cx.notify();
    }
    fn visible_rows(&self) -> Option<(usize, usize)> {
        let height = f32::from(self.bounds.size.height) as f64 - self.tokens.metrics.ruler as f64;
        let range = self
            .scene
            .as_ref()?
            .visible_range(self.view.scroll_y, height.max(0.));
        Some((range.start, range.end))
    }
    fn pump(&mut self, cx: &mut Context<Self>) {
        if self.sample_scheduled && matches!(self.queue.front(), Some(Job::Sample(_))) {
            return;
        }
        if self.busy {
            return;
        }
        let Some(mut job) = self.queue.pop_front() else {
            return;
        };
        let Some(mut client) = self.client.take() else {
            self.queue.push_front(job);
            return;
        };
        // Latest pointer sample wins while the kernel is busy.
        while matches!(job, Job::Sample(_)) && matches!(self.queue.front(), Some(Job::Sample(_))) {
            job = self.queue.pop_front().unwrap();
        }
        self.busy = true;
        let previous_scene = self.scene.clone();
        let preview_update = matches!(&job, Job::Sample(_));
        let (tokens, offline, sources) = (
            self.tokens.clone(),
            self.offline.clone(),
            self.sources.clone(),
        );
        cx.spawn(async move |this, cx| {
            let (client, effects, built, preview) = cx
                .background_executor()
                .spawn(async move {
                    let effects = run(&mut client, job).await;
                    let preview = client.layer_preview().cloned();
                    let built = if preview_update && preview.is_some() && previous_scene.is_some() {
                        let mut scene = (*previous_scene.unwrap()).clone();
                        scene.preview_times(&client.model, &preview.as_ref().unwrap().starts);
                        Some(Built {
                            scene: Arc::new(scene),
                            graph: None,
                            model: Arc::new(client.model.clone()),
                            wanted: vec![],
                        })
                    } else {
                        let mut built = build(&client, &tokens, offline, sources);
                        if let (Some(built), Some(preview)) = (&mut built, &preview) {
                            Arc::make_mut(&mut built.scene)
                                .preview_times(&client.model, &preview.starts);
                        }
                        built
                    };
                    remember_shown(&mut client, &built);
                    (client, effects, built, preview)
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.client = Some(client);
                s.busy = false;
                if let Some(preview) = preview {
                    s.delegate.timing_preview(Some(preview.snapshot), cx);
                    s.viewer_previewing = true;
                } else if s.viewer_previewing {
                    s.delegate.timing_preview(None, cx);
                    s.viewer_previewing = false;
                }
                s.install(built, cx);
                match effects {
                    Ok(effects) => {
                        for effect in effects {
                            match effect {
                                HostEffect::Seek(time) => s.delegate.seek(time, cx),
                                HostEffect::FitCurves(first, last) => s.fit_span(first, last, cx),
                                other => s.delegate.effect(other, cx),
                            }
                        }
                    }
                    Err(error) => s.delegate.report(error, cx),
                }
                s.pump(cx);
            });
        })
        .detach();
    }
    fn install(&mut self, built: Option<Built>, cx: &mut Context<Self>) {
        let Some(built) = built else {
            return;
        };
        for key in built.wanted {
            if self.reading.insert(key.clone()) {
                // Peaks are a full decode: a background task, then one rebuild.
                cx.spawn(async move |this, cx| {
                    let read = key.clone();
                    let wait = cx
                        .background_executor()
                        .spawn(async move { crate::waveforms::read(&read) })
                        .await;
                    let _ = wait.recv().await;
                    let _ = this.update(cx, |s, cx| {
                        s.reading.remove(&key);
                        s.published(cx);
                    });
                })
                .detach();
            }
        }
        self.model = built.model;
        self.scene = Some(built.scene.clone());
        self.graph = steady_graph(&self.gesture, self.graph.as_deref(), built.graph.clone());
        // A range the wheel panned to stays for its view.
        let panned = match (&self.graph, &self.graph_view) {
            (Some(g), Some((view, min, max))) if g.view == *view => {
                Some(Some(Arc::new((**g).clone().with_range(*min, *max))))
            }
            (_, Some(_)) => Some(None),
            _ => None,
        };
        match panned {
            Some(Some(graph)) => self.graph = Some(graph),
            Some(None) => self.graph_view = None,
            None => {}
        }
        self.graph_fitted = built.graph;
        self.outline.set_scene(Some(built.scene));
        self.frame = None;
        self.clamp_scroll();
        let duration = self.model.index.duration;
        if duration > 0. && self.bounds.size.width > px(0.) {
            let width = (f32::from(self.bounds.size.width) as f64 - self.view.layer_column).max(1.);
            let fit = ((width - FIT_ROOM).max(1.) / duration).clamp(PPS.0, PPS.1);
            // The view started at 0 and showed all of the old length.
            let whole = |d: f64| {
                d > 0. && self.view.scroll_time.abs() < 1e-9 && width / self.view.pps >= d * 0.99
            };
            if !self.fitted {
                self.fitted = true;
                self.view.pps = fit;
            } else if self.fit_duration > 0.
                && (self.fit_duration - duration).abs() > 1e-9
                && whole(self.fit_duration)
            {
                // A shorter or longer composition: a view that showed the
                // whole old length shows the whole new one (the ruler and bars
                // refit); a view the user zoomed into or scrolled stays.
                self.view.pps = fit;
            }
            self.fit_duration = duration;
        }
        self.changed(false, cx);
    }
    fn clamp_scroll(&mut self) {
        let Some(scene) = &self.scene else {
            return;
        };
        let height = f32::from(self.bounds.size.height) as f64 - self.tokens.metrics.ruler as f64;
        let limit = (scene.content_height() - height).max(0.);
        self.view.scroll_y = self.view.scroll_y.clamp(0., limit);
    }
    fn viewport(&self) -> Option<Viewport> {
        let (w, h) = (
            f32::from(self.bounds.size.width) as f64,
            f32::from(self.bounds.size.height) as f64,
        );
        (w > self.view.layer_column && h > 0.).then_some(Viewport {
            width: w,
            height: h,
            scroll_y: self.view.scroll_y,
            scroll_time: self.view.scroll_time,
            pps: self.view.pps,
            layer_column: self.view.layer_column,
        })
    }
    fn prepared(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) -> Option<Arc<Frame>> {
        let mut moved = self.bounds != bounds;
        if moved {
            self.bounds = bounds;
            self.frame = None;
            if !self.fitted && self.model.index.duration > 0. {
                self.fit_time(cx);
            }
            self.clamp_scroll();
            // A view restored or resized past the timeline's end comes back
            // onto its time.
            self.clamp_time();
        }
        if self.frame.is_none() {
            let shown = (self.view.pps, self.view.scroll_time, self.view.layer_column);
            moved |= shown != self.shown;
            self.shown = shown;
            let (scene, viewport) = (self.scene.as_ref()?, self.viewport()?);
            let _span = crate::diag::span("timeline.prepare");
            self.frame = scene.prepare(viewport, &self.tokens).ok().map(Arc::new);
            if let Some(frame) = self.frame.clone() {
                self.ask_painters(&frame, cx);
            }
            self.tell_service(cx);
        }
        if moved {
            // Paused, nothing else moves the playhead layer: it stayed put
            // on screen while the ruler and clips scrolled under it.
            let delegate = self.delegate.clone();
            cx.defer(move |cx| delegate.view_moved(cx));
        }
        self.frame.clone()
    }
    /// v1–3's `timeline` service: `pps`, `scrollT`, `scrollY`, `graph`,
    /// `keySelectionActive`, and each visible row's layer with its top and
    /// bottom in window points (`layerAtPoint`).
    fn service_facts(&self) -> serde_json::Value {
        use pm_panel_timeline::{RowId, v4::parse_row};
        let b = self.bounds;
        let (x, y) = (f32::from(b.origin.x) as f64, f32::from(b.origin.y) as f64);
        let (w, h) = (
            f32::from(b.size.width) as f64,
            f32::from(b.size.height) as f64,
        );
        let ruler = self.tokens.metrics.ruler as f64;
        let mut rows = vec![];
        if let (Some(scene), Some((start, end))) = (&self.scene, self.visible_rows()) {
            for i in start..end {
                let Some((row, top, bottom)) = scene.span(i) else {
                    break;
                };
                let layer = match parse_row(&row.id) {
                    Ok(RowId::Layer(id)) => id,
                    Ok(RowId::Property { layer_id, .. } | RowId::Effect { layer_id, .. }) => layer_id,
                    Err(_) => continue,
                };
                let at = |v: f64| ((y + ruler + v - self.view.scroll_y) * 100.).round() / 100.;
                rows.push(serde_json::json!([layer, at(top), at(bottom)]));
            }
        }
        serde_json::json!({
            "graph": self.model.graph_display != pm_panel_timeline::GraphDisplay::DopeSheet,
            "pps": self.view.pps,
            "scrollT": self.view.scroll_time,
            "scrollY": self.view.scroll_y,
            "keySelectionActive": self.model.key_selection_active,
            "rect": [x, y, w, h],
            "ruler": ruler,
            "column": self.view.layer_column,
            "rows": rows,
        })
    }
    fn tell_service(&mut self, cx: &mut Context<Self>) {
        let facts = self.service_facts();
        if self.told.as_ref() != Some(&facts) {
            self.told = Some(facts.clone());
            let delegate = self.delegate.clone();
            cx.defer(move |cx| delegate.view_facts(facts, cx));
        }
    }
    /// The v1 `timeline` service's writes from the window's realm: its view
    /// (`pps`, `scrollT`, `scrollY`), `frameView` and `reveal`. `graph` is the
    /// timeline panel's view (`Builtins::timeline_action`).
    pub fn service_request(&mut self, request: &serde_json::Value, cx: &mut Context<Self>) {
        let number = |key: &str| request[key].as_f64();
        if ["pps", "scrollT", "scrollY"]
            .iter()
            .any(|k| number(k).is_some())
        {
            self.set_view(number("pps"), number("scrollT"), number("scrollY"), cx);
        }
        if request["frameView"] == true {
            self.fit_time(cx);
        }
        if let Some(layer) = request["reveal"]["layer"].as_str() {
            let keys = request["reveal"]["keys"].as_array().map_or(vec![], |keys| {
                keys.iter()
                    .filter_map(|k| k.as_str().map(str::to_owned))
                    .collect()
            });
            self.reveal(layer.to_owned(), keys, cx);
        }
    }
    /// The service's writable `pps`, `scrollT` and `scrollY`, as main's fields.
    pub fn set_view(
        &mut self,
        pps: Option<f64>,
        scroll_time: Option<f64>,
        scroll_y: Option<f64>,
        cx: &mut Context<Self>,
    ) {
        if let Some(pps) = pps.filter(|p| p.is_finite()) {
            self.view.pps = pps.clamp(PPS.0, PPS.1);
            self.fitted = true;
        }
        if let Some(time) = scroll_time.filter(|t| t.is_finite()) {
            self.view.scroll_time = time;
        }
        if let Some(y) = scroll_y.filter(|y| y.is_finite()) {
            self.view.scroll_y = y.max(0.);
            self.clamp_scroll();
        }
        self.changed(true, cx);
    }
    /// `TimelineService.reveal`: open `layer` and show `keys` too.
    pub fn reveal(&mut self, layer: String, keys: Vec<String>, cx: &mut Context<Self>) {
        self.queue.push_back(Job::Reveal { layer, keys });
        self.pump(cx);
    }
    /// The bottom bar's height: what the body keeps under the tracks.
    /// The graph's values at the playhead `time` (v1's dots and readouts on
    /// the head), in window points; none while the graph is not shown.
    pub fn graph_marks(&self, time: f64) -> Vec<graph_render::Mark> {
        let (Some(area), Some(v), Some(graph)) = (self.graph_area(), self.viewport(), self.graph.as_ref())
        else {
            return vec![];
        };
        let (dx, dy) = (
            f32::from(self.bounds.origin.x) as f64,
            f32::from(self.bounds.origin.y) as f64,
        );
        let mut marks = graph.playhead_marks(v, area, time, &self.tokens);
        for m in &mut marks {
            (m.x, m.y) = (m.x + dx, m.y + dy);
            m.label = m.label.map(|l| l + dy);
        }
        marks
    }
    pub fn bar_height(&self) -> f64 {
        f32::from(self.body.size.height - self.bounds.size.height).max(0.) as f64
    }
    /// Properties' Show in Graph Editor: the graph view, on `channel`.
    pub fn show_graph(
        &mut self,
        layer: String,
        channel: String,
        keys: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.queue.push_back(Job::ShowGraph {
            layer,
            channel,
            keys,
        });
        self.pump(cx);
    }
    /// Playing, the view turns a page to keep the head in sight, as v1: once
    /// the head is within 50 points (or a tenth of the tracks) of the right
    /// edge it comes back a fifth of the way in; behind the left edge (a
    /// loop, or playing backwards) it goes to four fifths. Paging rather than
    /// pinning the head keeps everything else still between pages.
    pub fn follow_playhead(&mut self, time: f64, cx: &mut Context<Self>) {
        // A scrub or drag of the person's own leads the view.
        if !matches!(self.gesture, Gesture::Idle) {
            return;
        }
        let Some(width) = self.time_width() else {
            return;
        };
        let pps = self.view.pps;
        let Some(at) = follow_page((time - self.view.scroll_time) * pps, width) else {
            return;
        };
        let before = self.view.scroll_time;
        self.view.scroll_time = time - at / pps;
        self.clamp_time();
        if self.view.scroll_time != before {
            self.changed_while_hot(cx);
        }
    }
    pub fn time_mapping(&self) -> (f64, f64, f64) {
        self.shown
    }
    /// A clip painter arrived or left: ask again on the next frame.
    pub fn repaint_decorations(&mut self, cx: &mut Context<Self>) {
        self.painted = None;
        self.frame = None;
        cx.notify();
    }
    /// Extension painters run when the visible clips change (scroll, zoom,
    /// edits), never per playback frame; their shapes land asynchronously.
    fn ask_painters(&mut self, frame: &Frame, cx: &mut Context<Self>) {
        let revision = self.delegate.painters_revision(cx);
        if revision == 0 {
            if !self.decorations.is_empty() {
                self.decorations = Default::default();
            }
            return;
        }
        let clips: Vec<serde_json::Value> = frame
            .ops
            .iter()
            .filter_map(|op| {
                let id = op.id.strip_prefix("clip:")?;
                Some(serde_json::json!({"id":id,"layerId":id,
                    "width":(op.bounds.width * 10.).round() / 10.,"height":(op.bounds.height * 10.).round() / 10.,
                    "from":op.detail["from"],"duration":op.detail["duration"]}))
            })
            .collect();
        let key = (
            revision,
            serde_json::Value::Array(clips.clone()).to_string(),
        );
        // A painter that missed the deadline (a cold realm) is asked again on
        // a later frame, at most every half second.
        let retry = self
            .late
            .is_some_and(|at| at.elapsed() >= std::time::Duration::from_millis(500));
        if self.painting || (self.painted.as_ref() == Some(&key) && !retry) {
            return;
        }
        let asked = std::time::Instant::now();
        self.painting = true;
        self.painted = Some(key);
        let weak = cx.entity().downgrade();
        self.delegate.clone().paint_clips(
            serde_json::Value::Array(clips),
            Box::new(move |shapes, complete, cx| {
                let _ = weak.update(cx, |s, cx| {
                    s.painting = false;
                    s.late = (!complete).then_some(asked);
                    s.decorations = Arc::new(shapes);
                    s.frame = None;
                    cx.notify();
                });
            }),
            cx,
        );
    }
    fn local(&self, position: Point<Pixels>) -> (f64, f64) {
        let at = position - self.bounds.origin;
        (f32::from(at.x) as f64, f32::from(at.y) as f64)
    }
    /// Where media dropped at `position` lands: the frame under the pointer
    /// (the layer column counts as the first time shown), and its place in
    /// the stack ([`Scene::drop_before`], v1.2.0's rule).
    fn media_slot(&self, position: Point<Pixels>) -> (f64, Option<usize>, f64) {
        let (x, y) = self.local(position);
        let at = frame_time(
            &self.model,
            self.time_at(x.max(self.view.layer_column)).max(0.),
        );
        let ruler = self.tokens.metrics.ruler as f64;
        let content = y.max(ruler) - ruler + self.view.scroll_y;
        let layers = self.model.index.layers.len();
        let (index, top) = match self.scene.as_ref().map(|s| (s.drop_before(content), s)) {
            Some((pm_panel_timeline::render::DropSlot::Top, _)) | None => (Some(0), 0.),
            Some((pm_panel_timeline::render::DropSlot::Before(Some(id)), scene)) => (
                self.model.index.layer_ordinal(&id),
                scene
                    .scroll_for(&pm_panel_timeline::v4::row_id(
                        &pm_panel_timeline::RowId::Layer(id),
                    ))
                    .unwrap_or(0.),
            ),
            Some((pm_panel_timeline::render::DropSlot::Before(None), scene)) => {
                (Some(layers), scene.content_height())
            }
        };
        (at, index, ruler + top - self.view.scroll_y)
    }
    fn media_moved(&mut self, bounds: Bounds<Pixels>, at: Point<Pixels>, cx: &mut Context<Self>) {
        let over = bounds.contains(&at).then_some(at);
        if over != self.media_over {
            self.media_over = over;
            cx.notify();
        }
    }
    /// The layer whose row is under `position` (a property row's layer) and
    /// that row's top in the view: where a dropped effect goes.
    fn layer_under(&self, position: Point<Pixels>) -> Option<(String, f64)> {
        use pm_panel_timeline::{RowId, v4};
        let (_, y) = self.local(position);
        let ruler = self.tokens.metrics.ruler as f64;
        if y < ruler {
            return None;
        }
        let scene = self.scene.as_ref()?;
        let row = scene.row_at(y - ruler + self.view.scroll_y)?;
        let layer = match v4::parse_row(&row.id).ok()? {
            RowId::Layer(id) => id,
            RowId::Property { layer_id, .. } | RowId::Effect { layer_id, .. } => layer_id,
        };
        let top = scene.scroll_for(&v4::row_id(&RowId::Layer(layer.clone())))?;
        Some((layer, ruler + top - self.view.scroll_y))
    }
    fn effect_moved(&mut self, bounds: Bounds<Pixels>, at: Point<Pixels>, cx: &mut Context<Self>) {
        let over = bounds.contains(&at).then_some(at);
        if over != self.effect_over {
            self.effect_over = over;
            cx.notify();
        }
    }
    fn drop_media(&mut self, what: Dropped, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.media_over = None;
        let (at, index, _) = self.media_slot(position);
        self.delegate.drop_media(MediaDrop { at, index, what }, cx);
        cx.notify();
    }
    fn time_at(&self, x: f64) -> f64 {
        self.view.scroll_time + (x - self.view.layer_column) / self.view.pps
    }
    /// The plot: under the graph's legend strip, clear of its foot (v1's
    /// `top + 42`, `H - 16`), so keys at the value range's ends never clip.
    /// Values map into this rect.
    fn graph_area(&self) -> Option<Rect> {
        self.graph_region().map(graph_render::plot)
    }
    /// The graph's whole region under the ruler (lower half in Dope Sheet and Graph).
    fn graph_region(&self) -> Option<Rect> {
        let m = self.tokens.metrics;
        let (w, h) = (
            f32::from(self.bounds.size.width) as f64,
            f32::from(self.bounds.size.height) as f64,
        );
        let ruler = m.ruler as f64;
        let body = (h - ruler).max(1.);
        let x = self.view.layer_column;
        match self.model.graph_display {
            GraphDisplay::DopeSheet => None,
            GraphDisplay::Graph => Some(Rect::new(x, ruler, w - x, body)),
            GraphDisplay::Combined => {
                // The lower half, from the first row edge there: the graph
                // never cuts a strip or a row of keys in two.
                let half = ruler + body / 2.;
                let scroll = self.view.scroll_y;
                let top = self
                    .scene
                    .as_ref()
                    .and_then(|scene| scene.edge_from(half - ruler + scroll))
                    .map(|edge| edge - scroll + ruler)
                    .filter(|top| *top - half <= m.track_row as f64)
                    .unwrap_or(half);
                Some(Rect::new(x, top, w - x, (h - top).max(1.)))
            }
        }
    }
    /// The work-area handle under a point of the ruler (panel space): its
    /// grab zone is `WORK_HANDLE_REACH` either side of the edge, the ruler's
    /// full height; when both edges are that close, the nearer one (v1's
    /// workAreaHit).
    fn work_handle_at(&self, x: f64, y: f64) -> Option<Spec> {
        let frame = self.frame.as_ref()?;
        if y < 0. || y >= self.tokens.metrics.ruler as f64 || x < self.view.layer_column {
            return None;
        }
        let edge = |id: &str| {
            frame
                .ops
                .iter()
                .find(|o| o.id == id)
                .map(|o| (x - (o.bounds.x + o.bounds.width / 2.)).abs())
        };
        let near = |d: Option<f64>| d.filter(|d| *d <= WORK_HANDLE_REACH);
        match (near(edge("work-in")), near(edge("work-out"))) {
            (Some(a), Some(b)) => Some(if a <= b { Spec::WorkIn } else { Spec::WorkOut }),
            (Some(_), None) => Some(Spec::WorkIn),
            (None, Some(_)) => Some(Spec::WorkOut),
            (None, None) => None,
        }
    }
    fn mods(modifiers: Modifiers) -> Mods {
        Mods {
            shift: modifiers.shift,
            alt: modifiers.alt,
            meta: modifiers.platform,
            control: modifiers.control,
        }
    }
    fn on_down(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let (x, y) = self.local(event.position);
        // A new press starts with no graph drag state.
        self.transform = None;
        self.graph_axis = false;
        self.readout = None;
        if let Some(bar) = self.bar()
            && let Some(part) = self.bar_part(x, y)
        {
            self.set_skimmer(None, cx);
            self.press_bar(part, bar, x, y, event.click_count, cx);
            return;
        }
        if y >= f32::from(self.bounds.size.height) as f64 {
            return;
        }
        // A work-area handle takes the press before anything else (no seek,
        // no scrub): the nearer one when both are close.
        if let Some(spec) = self.work_handle_at(x, y) {
            self.set_skimmer(None, cx);
            self.gesture = Gesture::Pressed(Press {
                origin: event.position,
                spec: Some(spec),
                release: None,
            });
            return;
        }
        // Final Cut: a click moves the playhead to the skimmer; the gesture
        // (selection, a drag) goes on from there, without a skimmer. A key, a
        // direction handle or a transform grip under the pointer takes the
        // press as it stands: the playhead stays where it is.
        let grabs = self.frame.as_ref().is_some_and(|f| f.hit_key(x, y, 3.).is_some())
            || self.graph_point_at(x, y);
        if let Some(at) = self.skimmer
            && !grabs
        {
            self.delegate.seek(at, cx);
        }
        self.set_skimmer(None, cx);
        let (x, y) = self.local(event.position);
        let Some(frame) = self.frame.clone() else {
            return;
        };
        let m = self.tokens.metrics;
        let column = self.view.layer_column;
        if (x - column).abs() <= m.hit as f64 / 4. && y >= m.ruler as f64 {
            if event.click_count == 2 {
                self.view.layer_column = default_layer_column(&m);
                self.view.column_chosen = false;
                self.changed(true, cx);
            } else {
                self.gesture = Gesture::Column;
            }
            return;
        }
        if x < column {
            return;
        }
        let mods = Self::mods(event.modifiers);
        let additive = !(mods.alt && mods.command()) && (mods.shift || mods.command());
        if y < m.ruler as f64 {
            // v1: the work area's bar (its thin strip at the top of the
            // ruler) moves it on a plain drag; ⌥ moves it from anywhere on
            // the ruler; the rest of the ruler scrubs.
            if frame.hit_op("work-area", x, y, 0.).is_some() && (mods.alt || y <= WORK_BAR_STRIP) {
                self.gesture = Gesture::Pressed(Press {
                    origin: event.position,
                    spec: Some(Spec::WorkMove),
                    release: None,
                });
                return;
            }
            let time = self.time_at(x).clamp(0., self.model.index.duration);
            if let Ok(mut scrub) = Scrub::begin(self.model.index.clone(), time)
                && let Ok(HostEffect::Seek(time)) =
                    scrub.update(time, event.modifiers.shift, self.snap_radius())
            {
                self.delegate.seek(frame_time(&self.model, time), cx);
                self.gesture = Gesture::Scrub(scrub);
            }
            return;
        }
        if let Some(region) = self.graph_region()
            && y >= region.y
        {
            self.press_graph(event, x, y, additive, cx);
            return;
        }
        // A caption cue: selected (and edited on double-click) by itself,
        // in step with the Captions panel; the clip isn't dragged.
        if let Some((layer, cue)) = frame
            .hit_op("cue:", x, y, 0.)
            .and_then(|o| pm_panel_timeline::render::parse_cue_op(&o.id))
        {
            let mode = if event.modifiers.shift {
                "extend"
            } else if event.modifiers.platform {
                "toggle"
            } else {
                "replace"
            };
            let verb = if event.click_count >= 2 { "edit" } else { mode };
            self.delegate
                .command(&format!("captions.cue:{verb}:{layer}\u{1f}{cue}"), cx);
            return;
        }
        let slop = ((m.hit - m.keyframe) as f64 / 2.).max(0.);
        if let Some((on, key)) = frame.hit_layer_key(x, y, slop)
            && let Some((layer, channel, hit)) = self.model.index.key_on(&on, &key)
        {
            // v1: a double-click on a key opens its value editor, for the
            // whole selection when the key is part of it (the first click
            // selected it).
            if event.click_count == 2 {
                self.delegate.command("keyframeValue", cx);
                return;
            }
            let selected = self.model.selection.key_selected(&on, &key);
            let grabbed = layer.from + hit.time;
            let (layer, channel) = (layer.id.clone(), channel.path.clone());
            self.queue.push_back(Job::SelectKey {
                layer: layer.clone(),
                channel: channel.clone(),
                key: key.clone(),
                additive,
                dragging: true,
            });
            // v1: ⌥ stretches the selected keys' timing from the far end
            // when the first or last of them is grabbed.
            let spec = if mods.alt && mods.command() {
                Spec::QuickOffsetKeys
            } else if mods.alt && !additive && !mods.command() {
                Spec::StretchKeys { grabbed }
            } else {
                Spec::Keys
            };
            self.gesture = Gesture::Pressed(Press {
                origin: event.position,
                spec: Some(spec),
                release: (additive && selected).then_some(Job::SelectKey {
                    layer,
                    channel,
                    key,
                    additive,
                    dragging: false,
                }),
            });
            self.pump(cx);
            return;
        }
        // v1 textStagger.hit runs before a clip body's slide.
        if let Some(op) = frame.ops.iter().rev().find(|op| {
            frame.hit_clip(x, y).is_none_or(|(_, edge)| edge.is_none())
                && (op.shape == "stagger"
                    && op.detail["locked"] != true
                    && y >= op.detail["hitTop"].as_f64().unwrap_or(f64::INFINITY)
                    && y <= op.detail["hitBottom"].as_f64().unwrap_or(f64::NEG_INFINITY)
                    && (x - (op.bounds.x + op.bounds.width)).abs() < 5.
                    || op.shape == "stagger"
                        && op.detail["locked"] != true
                        && y >= op.detail["hitTop"].as_f64().unwrap_or(f64::INFINITY)
                        && y <= op.detail["hitBottom"].as_f64().unwrap_or(f64::NEG_INFINITY)
                        && x >= op.bounds.x - 2.
                        && x <= op.bounds.x + op.bounds.width)
        }) {
            if let Ok(band) = serde_json::from_value(op.detail["band"].clone()) {
                let end = (x - (op.bounds.x + op.bounds.width)).abs() < 5.;
                let band: pm_panel_timeline::stagger::Band = band;
                self.queue
                    .push_back(Job::SelectLayer(band.layer.clone(), mods));
                self.gesture = Gesture::Pressed(Press {
                    origin: event.position,
                    spec: Some(Spec::TextStagger { band, end }),
                    release: None,
                });
                self.pump(cx);
                return;
            }
        }
        if let Some((layer, edge)) = frame.hit_clip(x, y) {
            let layer = layer.to_owned();
            if self
                .model
                .index
                .layer(&layer)
                .is_some_and(|l| l.effectively_locked)
            {
                self.queue.push_back(Job::SelectLayer(layer, mods));
                self.pump(cx);
                return;
            }
            if event.click_count == 2 {
                if let Some(id) = self.model.index.layer(&layer).and_then(|l| {
                    (l.kind == "precomp")
                        .then(|| l.definition.clone())
                        .flatten()
                }) {
                    self.delegate.effect(HostEffect::OpenComposition(id), cx);
                }
                return;
            }
            let tracks = self.model.mode == TimelineMode::Tracks;
            // A chosen edit tool works in both modes; in Tracks the
            // modifiers reach the same edits with the Selection tool.
            let tool = self.model.edit_tool;
            let spec = match edge {
                _ if tool == EditTool::Slip => Spec::Slip,
                _ if tool == EditTool::Slide => Spec::Slide,
                Some(edge) => {
                    let edge = match edge {
                        pm_panel_timeline::render::Edge::In => Edge::In,
                        pm_panel_timeline::render::Edge::Out => Edge::Out,
                    };
                    if tool == EditTool::Ripple || tracks && mods.alt {
                        Spec::Ripple(edge)
                    } else if tool == EditTool::Roll || tracks && mods.control {
                        Spec::Roll(edge)
                    } else {
                        Spec::Trim(edge)
                    }
                }
                None if mods.command() && mods.alt => Spec::Offset,
                None if tracks && mods.control => Spec::Slip,
                None if tracks && mods.alt && mods.shift => Spec::Slide,
                // Option-drag duplicates, as in Premiere, Final Cut and
                // Resolve (Slide took Option alone before; it has Shift now).
                None if mods.alt => Spec::Duplicate,
                None => Spec::Layers,
            };
            let selected = self.model.selection.contains(&layer);
            // Pro-edit modifiers choose the tool; they must not toggle selection.
            let picking = if matches!(
                spec,
                Spec::Ripple(_) | Spec::Roll(_) | Spec::Slip | Spec::Slide | Spec::Duplicate
            ) {
                Mods::default()
            } else {
                mods
            };
            self.queue
                .push_back(Job::SelectLayer(layer.clone(), picking));
            self.gesture = Gesture::Pressed(Press {
                origin: event.position,
                spec: Some(spec),
                release: (selected && !additive && self.model.selection.layers().len() > 1).then(
                    || {
                        Job::Apply(Plan {
                            calls: vec![
                                SelectionCall::Layers {
                                    ids: vec![layer],
                                    add: false,
                                }
                                .into(),
                            ],
                            ..Default::default()
                        })
                    },
                ),
            });
            self.pump(cx);
            return;
        }
        self.gesture = Gesture::Marquee {
            session: MarqueeSession::begin(&self.model, additive, false),
            origin: event.position,
            at: event.position,
        };
    }
    /// The layer header was pressed (the JSX outline): a reorder
    /// starts here if the pointer travels.
    pub fn press_layer_row(&mut self, layer: String, origin: Point<Pixels>) {
        if matches!(self.gesture, Gesture::Idle) {
            self.gesture = Gesture::LayerPress { layer, origin };
        }
    }
    /// v1's `layerDrop` over the scene: the target layer, before / after /
    /// into a group (its middle half, past the disclosure), and the cue.
    fn layer_drop(&self, position: Point<Pixels>) -> Option<LayerDrop> {
        use pm_panel_timeline::DropMode;
        let Gesture::Reorder { moving, .. } = &self.gesture else {
            return None;
        };
        let (x, y) = self.local(position);
        let (frame, scene) = (self.frame.as_ref()?, self.scene.as_ref()?);
        let at = scene.layer_row_at(frame.viewport, &self.tokens, y)?;
        let pm_panel_timeline::RowId::Layer(id) = pm_panel_timeline::v4::parse_row(&at.id).ok()?
        else {
            return None;
        };
        let index = &self.model.index;
        let layer = index.layer(&id)?;
        if layer.effectively_locked || index.expanded_ids(moving).contains(&id) {
            return None;
        }
        let m = self.tokens.metrics;
        let inside =
            layer.kind == "group" && (0.25..=0.75).contains(&at.fraction) && x >= 2. * m.hit as f64;
        let mode = if inside {
            DropMode::Inside
        } else if at.fraction < 0.5 {
            DropMode::Before
        } else {
            DropMode::After
        };
        Some(LayerDrop {
            target: id,
            mode,
            line: if mode == DropMode::Inside {
                at.top + at.height
            } else if mode == DropMode::After {
                at.end
            } else {
                at.top
            },
            left: m.inset as f64 + (at.depth + inside as usize) as f64 * m.tree_indent as f64,
            name: layer.name.to_string_lossy(),
        })
    }
    /// A reorder held past the list's top or bottom scrolls the rows.
    fn autoscroll_rows(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (_, y) = self.local(position);
        let top = self.tokens.metrics.ruler as f64 + 24.;
        let bottom = f32::from(self.bounds.size.height) as f64 - self.bar_height() - 24.;
        let over = if y < top {
            y - top
        } else if y > bottom {
            y - bottom
        } else {
            return;
        };
        self.view.scroll_y = (self.view.scroll_y + over.clamp(-48., 48.) * 0.25).max(0.);
        self.clamp_scroll();
        self.changed(false, cx);
    }
    pub fn press_parent(&mut self, layer: String, origin: Point<Pixels>) {
        let selected = self.model.selection.layers();
        let ids = if selected.contains(&layer) {
            selected.to_vec()
        } else {
            vec![layer]
        };
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| {
                self.model
                    .index
                    .layer(id)
                    .is_some_and(|l| !l.effectively_locked && l.kind != "audio")
            })
            .collect();
        if !ids.is_empty() {
            self.gesture = Gesture::ParentPick {
                ids,
                origin,
                at: origin,
                target: None,
                moved: false,
            };
        }
    }
    fn parent_target(&self, ids: &[String], position: Point<Pixels>) -> Option<String> {
        if !self.bounds.contains(&position) {
            return None;
        }
        let (_, y) = self.local(position);
        let frame = self.frame.as_ref()?;
        let row = self
            .scene
            .as_ref()?
            .layer_row_at(frame.viewport, &self.tokens, y)?;
        let RowId::Layer(id) = pm_panel_timeline::v4::parse_row(&row.id).ok()? else {
            return None;
        };
        // v1 wouldCycle also rejects an ancestor group, and a group's descendants.
        let ancestors = |start: &str| {
            let mut result = BTreeSet::new();
            let mut at = self
                .model
                .index
                .layer(start)
                .and_then(|l| l.group.as_deref());
            while let Some(group) = at {
                if !result.insert(group.to_owned()) {
                    break;
                }
                at = self
                    .model
                    .index
                    .layer(group)
                    .and_then(|l| l.group.as_deref());
            }
            result
        };
        if ids.iter().any(|child| ancestors(child).contains(&id)) {
            return None;
        }
        let mut candidate = Some(id.as_str());
        let mut seen = BTreeSet::new();
        if self.model.index.layer(&id)?.kind == "audio" {
            return None;
        }
        while let Some(parent) = candidate {
            if ids.iter().any(|child| child == parent) || !seen.insert(parent) {
                return None;
            }
            if ids.iter().any(|child| {
                self.model
                    .index
                    .layer(child)
                    .is_some_and(|l| l.kind == "group")
                    && ancestors(parent).contains(child)
            }) {
                return None;
            }
            candidate = self
                .model
                .index
                .layer(parent)
                .and_then(|l| l.parent.as_deref());
        }
        Some(id)
    }
    pub fn press_property(&mut self, layer: String, path: String, origin: Point<Pixels>) {
        self.gesture = Gesture::Pressed(Press {
            origin,
            spec: Some(Spec::Property { layer, path }),
            release: None,
        });
    }
    /// A press in the graph, in v1's order (`graphDown`): a transform grip,
    /// then the nearer of a handle and a key (a key wins within 4 points),
    /// then the selection box (moves the group), then a marquee. A
    /// double-click on a key edits its value.
    fn press_graph(
        &mut self,
        event: &MouseDownEvent,
        x: f64,
        y: f64,
        additive: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(region) = self.graph_region() else {
            return;
        };
        if y < region.y + graph_render::LEGEND {
            return;
        }
        let (graph, viewport) = (self.graph.clone(), self.viewport());
        let boxed = graph
            .as_ref()
            .zip(viewport)
            .and_then(|(g, v)| g.selection_box(v, region));
        if let (Some(g), Some(b)) = (graph.as_ref(), boxed)
            && g.transformable()
            && let Some(grip) = graph_render::grip_at(b, x, y)
            && let Some((t0, t1, v0, v1)) = g.selected_span()
        {
            let b = TransformBox {
                t0,
                t1,
                v0,
                v1,
                grip,
                centred: event.modifiers.platform || event.modifiers.control,
            };
            let (anchor_time, anchor_value) = b.anchor();
            self.transform = Some(b);
            self.gesture = Gesture::Pressed(Press {
                origin: event.position,
                spec: Some(Spec::Transform {
                    anchor_time,
                    anchor_value,
                }),
                release: None,
            });
            return;
        }
        let ops = self.graph_ops(None).unwrap_or_default();
        let nearest = |prefix: &str| {
            ops.iter()
                .filter(|o| o.id.starts_with(prefix))
                .map(|o| {
                    let (cx, cy) = (
                        o.bounds.x + o.bounds.width / 2.,
                        o.bounds.y + o.bounds.height / 2.,
                    );
                    ((cx - x).hypot(cy - y), o.id[prefix.len()..].to_owned())
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
        };
        let point = nearest("graph-key:");
        let handle = nearest("graph-handle:");
        let pd = point.as_ref().map_or(f64::INFINITY, |p| p.0);
        let hd = handle.as_ref().map_or(f64::INFINITY, |h| h.0);
        let prefer_point =
            pd < graph_render::POINT_HIT && (hd >= graph_render::HANDLE_HIT || hd + 4. >= pd);
        if hd < graph_render::HANDLE_HIT
            && !prefer_point
            && let Some((key, side)) = handle.and_then(|(_, h)| {
                let (key, side) = h.rsplit_once(':')?;
                let side = match side {
                    "in" => pm_kernel::commands::keyframes::EaseSide::In,
                    _ => pm_kernel::commands::keyframes::EaseSide::Out,
                };
                Some((key.to_owned(), side))
            })
        {
            self.gesture = Gesture::Pressed(Press {
                origin: event.position,
                spec: Some(Spec::Handle { key, side }),
                release: None,
            });
            return;
        }
        if pd >= graph_render::POINT_HIT {
            if boxed
                .is_some_and(|b| x >= b.x && x <= b.x + b.width && y >= b.y && y <= b.y + b.height)
            {
                // Inside the box: the selected keys move together.
                self.graph_axis = true;
                self.gesture = Gesture::Pressed(Press {
                    origin: event.position,
                    spec: Some(Spec::GraphKeys),
                    release: None,
                });
            } else {
                self.gesture = Gesture::Marquee {
                    session: MarqueeSession::begin(&self.model, additive, true),
                    origin: event.position,
                    at: event.position,
                };
            }
            return;
        }
        let Some(key) = point.map(|(_, id)| id) else {
            return;
        };
        let Some((layer, channel, _)) = graph
            .as_ref()
            .and_then(|g| g.key_layer(&key))
            .and_then(|on| self.model.index.key_on(on, &key))
        else {
            return;
        };
        let (layer, channel) = (layer.id.clone(), channel.path.clone());
        let selected = self.model.selection.key_selected(&layer, &key);
        self.queue.push_back(Job::SelectKey {
            layer: layer.clone(),
            channel: channel.clone(),
            key: key.clone(),
            additive,
            dragging: true,
        });
        if event.click_count >= 2 {
            // v1: a double-click on a key edits its value.
            self.queue.push_back(Job::Menu {
                context: MenuContext::Keys { ids: vec![key] },
                action: "keys.value".into(),
                time: self.delegate.time(cx),
            });
            self.pump(cx);
            return;
        }
        self.graph_axis = true;
        self.gesture = Gesture::Pressed(Press {
            origin: event.position,
            spec: Some(Spec::GraphKeys),
            // ⇧-click on a selected key takes it out of the selection on
            // release; a ⇧-drag moves the selection with it.
            release: (additive && selected).then_some(Job::SelectKey {
                layer,
                channel,
                key,
                additive,
                dragging: false,
            }),
        });
        self.pump(cx);
    }
    /// The graph drawn into its region; `playhead` marks each curve's value
    /// there.
    fn graph_point_at(&self, x: f64, y: f64) -> bool {
        let Some(region) = self.graph_region() else { return false; };
        if y < region.y + graph_render::LEGEND { return false; }
        if self.graph.as_ref().zip(self.viewport()).and_then(|(g,v)| g.selection_box(v,region))
            .is_some_and(|b| graph_render::grip_at(b,x,y).is_some()) { return true; }
        self.graph_ops(None).is_some_and(|ops| ops.iter().any(|o|
            (o.id.starts_with("graph-key:") || o.id.starts_with("graph-handle:")) &&
            (x-o.bounds.x-o.bounds.width/2.).hypot(y-o.bounds.y-o.bounds.height/2.) < graph_render::POINT_HIT))
    }
    fn graph_ops(&self, playhead: Option<f64>) -> Option<Vec<Op>> {
        self.graph
            .as_ref()?
            .prepare(
                self.viewport()?,
                self.graph_region()?,
                &self.tokens,
                playhead,
            )
            .ok()
    }
    fn sample(
        &self,
        origin: Point<Pixels>,
        at: Point<Pixels>,
        modifiers: Modifiers,
        cx: &App,
    ) -> DragSample {
        let delta = at - origin;
        let radius = self.snap_radius();
        let value_scale = self
            .graph_area()
            .zip(self.graph.as_ref())
            .map_or(0., |(area, graph)| graph.value_per_point(area.height));
        DragSample {
            time_delta: f32::from(delta.x) as f64 / self.view.pps,
            value_delta: -(f32::from(delta.y) as f64) * value_scale,
            time_scale: 1.,
            value_scale: 1.,
            anchor: None,
            handle_time: 0.,
            handle_value: 0.,
            modifiers: Self::mods(modifiers),
            acquire_tolerance: radius,
            release_tolerance: radius,
            playhead: self.delegate.time(cx),
        }
    }
    /// Snap reach in seconds: one keyframe glyph on screen, at any zoom.
    fn snap_radius(&self) -> f64 {
        self.tokens.metrics.keyframe as f64 / self.view.pps
    }
    fn on_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Gesture::ParentPick { ids, origin, .. } = &self.gesture {
            let target = self.parent_target(ids, event.position);
            let delta = event.position - *origin;
            let travel = f32::from(delta.x).hypot(f32::from(delta.y));
            if let Gesture::ParentPick {
                at,
                target: next,
                moved,
                ..
            } = &mut self.gesture
            {
                *at = event.position;
                *next = target;
                *moved |= travel >= self.tokens.metrics.panel_drag_threshold;
            }
            cx.notify();
            return true;
        }
        let held = match self.gesture {
            Gesture::Zoom => Some((Part::Zoom, 0.)),
            Gesture::Navigate { grab } => Some((Part::Time, grab)),
            Gesture::Rows { grab } => Some((Part::Rows, grab)),
            _ => None,
        };
        if let Some((part, grab)) = held {
            let (x, y) = self.local(event.position);
            self.drag_bar(part, x, y, grab, cx);
            return true;
        }
        if let Gesture::LayerPress { layer, origin } = &self.gesture {
            let delta = event.position - *origin;
            if f32::from(delta.x).hypot(f32::from(delta.y)) < 4. {
                return false;
            }
            // v1: the pressed layer joins the selection it moves with, or
            // becomes it; a locked layer (or group) is not reordered.
            let layer = layer.clone();
            let selected = self.model.selection.layers().to_vec();
            let moving = if selected.contains(&layer) {
                selected
            } else {
                self.queue.push_back(Job::Apply(Plan {
                    calls: vec![
                        SelectionCall::Layers {
                            ids: vec![layer.clone()],
                            add: false,
                        }
                        .into(),
                    ],
                    ..Default::default()
                }));
                vec![layer]
            };
            let index = &self.model.index;
            let locked = index
                .expanded_ids(&moving)
                .iter()
                .any(|id| index.layer(id).is_none_or(|l| l.effectively_locked));
            if !locked {
                self.cursor = CursorStyle::ClosedHand;
            }
            self.gesture = if locked {
                Gesture::Idle
            } else {
                Gesture::Reorder {
                    moving,
                    at: event.position,
                    drop: None,
                }
            };
            self.pump(cx);
            if matches!(self.gesture, Gesture::Reorder { .. }) {
                self.row_scroll = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(16))
                            .await;
                        let active = this
                            .update(cx, |s, cx| {
                                let Gesture::Reorder { at, .. } = s.gesture else {
                                    return false;
                                };
                                s.autoscroll_rows(at, cx);
                                let next = s.layer_drop(at);
                                if let Gesture::Reorder { drop, .. } = &mut s.gesture {
                                    *drop = next;
                                }
                                cx.notify();
                                true
                            })
                            .unwrap_or(false);
                        if !active {
                            break;
                        }
                    }
                }));
            }
        }
        if matches!(self.gesture, Gesture::Reorder { .. }) {
            self.autoscroll_rows(event.position, cx);
            let next = self.layer_drop(event.position);
            if let Gesture::Reorder { drop, at, .. } = &mut self.gesture {
                *at = event.position;
                *drop = next;
            }
            cx.notify();
            return true;
        }
        match &mut self.gesture {
            Gesture::Idle => return false,
            Gesture::Pressed(press) => {
                let delta = event.position - press.origin;
                let travel = if matches!(press.spec, Some(Spec::Property { .. })) {
                    f32::from(delta.x).abs()
                } else {
                    f32::from(delta.x).hypot(f32::from(delta.y))
                };
                if travel < self.tokens.metrics.drag_threshold
                    && !matches!(press.spec, Some(Spec::TextStagger { .. }))
                {
                    return true;
                }
                let origin = press.origin;
                let Some(spec) = press.spec.take() else {
                    self.gesture = Gesture::Idle;
                    return true;
                };
                let proedit = matches!(
                    spec,
                    Spec::Ripple(_) | Spec::Roll(_) | Spec::Slip | Spec::Slide
                );
                let readout = matches!(spec, Spec::Property { .. });
                let quick_count = match spec {
                    Spec::QuickOffsetKeys => self
                        .model
                        .index
                        .layers
                        .iter()
                        .filter(|l| {
                            l.channels.iter().any(|c| {
                                c.keys
                                    .iter()
                                    .any(|k| self.model.selection.keys().contains(&k.id))
                            })
                        })
                        .count(),
                    Spec::Offset => self.model.selection.layers().len(),
                    _ => 0,
                };
                let quick = (quick_count > 1).then_some((event.position, 0., quick_count));
                let mut quick_limits = Vec::new();
                if quick_count > 1 && matches!(spec, Spec::QuickOffsetKeys) {
                    let mut rank = 0;
                    for layer in &self.model.index.layers {
                        let times: Vec<_> = layer
                            .channels
                            .iter()
                            .flat_map(|c| &c.keys)
                            .filter(|k| self.model.selection.keys().contains(&k.id))
                            .map(|k| layer.from + k.time)
                            .collect();
                        if times.is_empty() {
                            continue;
                        }
                        let factor = rank as f64 / (quick_count - 1) as f64;
                        quick_limits.extend(times.into_iter().map(|time| (time, factor)));
                        rank += 1;
                    }
                }
                let handle = match &spec {
                    Spec::Handle { key, .. } => self
                        .graph
                        .as_ref()
                        .and_then(|g| g.key_layer(key))
                        .and_then(|on| self.model.index.key_on(on, key))
                        .map(|(l, _, _)| l.from),
                    _ => None,
                };
                let time = self.delegate.time(cx);
                // v1: a clip being moved is held (grabbing).
                if matches!(spec, Spec::Layers | Spec::Duplicate) {
                    self.cursor = CursorStyle::ClosedHand;
                }
                self.queue.push_back(Job::Begin(spec, time));
                self.gesture = Gesture::Dragging {
                    origin,
                    proedit,
                    readout,
                    quick,
                    quick_limits,
                    handle,
                };
            }
            _ => {}
        }
        match &mut self.gesture {
            Gesture::Dragging {
                origin,
                proedit,
                readout,
                quick,
                quick_limits,
                handle,
            } => {
                if let Some((at, total, _)) = quick {
                    *at = event.position;
                    *total = f32::from(event.position.x - origin.x) as f64 / self.view.pps;
                    // v1 drawQuickOffset reads the planner's bounded total.
                    for &(time, factor) in quick_limits.iter().filter(|(_, factor)| *factor > 0.) {
                        *total = total
                            .max((time.min(0.) - time) / factor)
                            .min((time.max(self.model.index.duration) - time) / factor);
                    }
                    cx.notify();
                }
                let (origin, proedit, readout, handle) = (*origin, *proedit, *readout, *handle);
                let mut sample = self.sample(origin, event.position, event.modifiers, cx);
                if let (Some(layer_from), Some(area), Some(graph)) =
                    (handle, self.graph_area(), self.graph.as_ref())
                {
                    let (x, y) = self.local(event.position);
                    sample.handle_time = self.time_at(x) - layer_from;
                    sample.handle_value = graph.value_at(area, y);
                }
                if readout {
                    let delta = event.position - origin;
                    if let Ok(value) = pm_surface::binding::scrub(pm_surface::binding::Scrub {
                        start: 0.,
                        delta: (f32::from(delta.x) as f64, f32::from(delta.y) as f64),
                        step: 1.,
                        speed: 0.5,
                        shift: event.modifiers.shift,
                        alt: event.modifiers.alt,
                        bounds: (-f64::MAX, f64::MAX),
                    }) {
                        sample.value_delta = value;
                    }
                }
                if proedit {
                    // Pro timing takes exact frames: the kernel never snaps subframes.
                    sample.time_delta = frame(sample.time_delta, self.model.index.fps);
                }
                let (lx, ly) = self.local(event.position);
                if let (Some(b), Some(area), Some(graph)) =
                    (self.transform, self.graph_area(), self.graph.as_ref())
                {
                    let d = event.position - origin;
                    let (ts, vs) = b.scales(
                        f32::from(d.x) as f64 / self.view.pps,
                        -(f32::from(d.y) as f64) * graph.value_per_point(area.height),
                    );
                    (sample.time_scale, sample.value_scale) = (ts, vs);
                    let (east, west, north, south) = b.sides();
                    let percent = |s: f64| format!("{}%", graph_render::value_text(s * 100., 1));
                    let mut parts = vec![];
                    if east || west {
                        parts.push(format!("T {}", percent(ts)));
                    }
                    if north || south {
                        parts.push(format!("V {}", percent(vs)));
                    }
                    self.readout = Some((parts.join("  ·  "), lx, ly));
                } else if self.graph_axis {
                    // ⇧ snaps (above) and keeps the drag on its main axis.
                    if event.modifiers.shift {
                        let d = event.position - origin;
                        if f32::from(d.x).abs() >= f32::from(d.y).abs() {
                            sample.value_delta = 0.;
                        } else {
                            sample.time_delta = 0.;
                        }
                    }
                    self.readout = Some((self.graph_readout(&sample), lx, ly));
                }
                self.autoscroll(event.position, cx);
                // Keep one sample for the next display refresh. A mouse-up
                // remains behind that sample, so the last position commits.
                if matches!(self.queue.back(), Some(Job::Sample(_))) {
                    self.queue.pop_back();
                }
                self.queue.push_back(Job::Sample(sample));
                if !self.sample_scheduled {
                    self.sample_scheduled = true;
                    cx.on_next_frame(window, |s, _, cx| {
                        s.sample_scheduled = false;
                        s.pump(cx);
                    });
                    window.request_animation_frame();
                }
            }
            Gesture::Scrub(scrub) => {
                let (x, _) = {
                    let at = event.position - self.bounds.origin;
                    (f32::from(at.x) as f64, 0.)
                };
                let time = self.view.scroll_time + (x - self.view.layer_column) / self.view.pps;
                let radius = self.tokens.metrics.keyframe as f64 / self.view.pps;
                if let Ok(HostEffect::Seek(time)) =
                    scrub.update(time, event.modifiers.shift, radius)
                {
                    let time = frame_time(&self.model, time);
                    self.delegate.seek(time, cx);
                }
            }
            Gesture::Marquee { at, .. } => {
                *at = event.position;
                self.marquee(cx);
                cx.notify();
            }
            Gesture::Pan { at } => {
                let delta = event.position - *at;
                *at = event.position;
                self.view.scroll_time -= f32::from(delta.x) as f64 / self.view.pps;
                self.clamp_time();
                self.changed(false, cx);
            }
            Gesture::Column => {
                let m = self.tokens.metrics;
                let width = f32::from(self.bounds.size.width) as f64;
                let x = f32::from(event.position.x - self.bounds.origin.x) as f64;
                // Never past the row's least width (its switches always fit);
                // never past the panel's edge, unless the panel is narrower.
                self.view.layer_column = x
                    .min((width - m.hit as f64).max(1.))
                    .max(least_layer_column(&m));
                self.view.column_chosen = true;
                self.changed(false, cx);
            }
            _ => {}
        }
        true
    }
    fn autoscroll(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let left = self.bounds.origin.x + px(self.view.layer_column as f32);
        let right = self.bounds.origin.x + self.bounds.size.width;
        let over = edge_overflow(f32::from(position.x), f32::from(left), f32::from(right));
        if over == 0. {
            return;
        }
        self.view.scroll_time = (self.view.scroll_time + over as f64 * 0.1 / self.view.pps)
            .max(self.model.index.minimum_time());
        self.changed(false, cx);
    }
    fn marquee(&mut self, cx: &mut Context<Self>) {
        let (origin, at, in_graph) = match &self.gesture {
            Gesture::Marquee {
                session,
                origin,
                at,
            } => (*origin, *at, session.in_graph()),
            _ => return,
        };
        let (a, b) = (origin - self.bounds.origin, at - self.bounds.origin);
        let (ax, ay, bx, by) = (
            f32::from(a.x.min(b.x)) as f64,
            f32::from(a.y.min(b.y)) as f64,
            f32::from(a.x.max(b.x)) as f64,
            f32::from(a.y.max(b.y)) as f64,
        );
        if bx - ax < self.tokens.metrics.drag_threshold as f64
            && by - ay < self.tokens.metrics.drag_threshold as f64
        {
            return;
        }
        let area = Rect::new(ax, ay, bx - ax, by - ay);
        let (Some(frame), Some(scene)) = (&self.frame, &self.scene) else {
            return;
        };
        let covered = scene.marquee(frame.viewport, &self.tokens, area);
        // In the graph a box picks the curves' keys it covers, as they are
        // drawn there (the rows under the graph are not shown).
        let keys = if in_graph {
            let graph = self.graph.clone();
            self.graph_ops(None)
                .unwrap_or_default()
                .iter()
                .filter_map(|o| {
                    let id = o.id.strip_prefix("graph-key:")?;
                    let (x, y) = (o.bounds.x + o.bounds.width / 2., o.bounds.y + o.bounds.height / 2.);
                    let inside = x >= area.x
                        && x <= area.x + area.width
                        && y >= area.y
                        && y <= area.y + area.height;
                    let layer = graph.as_ref()?.key_layer(id)?.to_owned();
                    inside.then(|| (layer, id.to_owned()))
                })
                .collect()
        } else {
            covered.keys
        };
        let Gesture::Marquee { session, .. } = &mut self.gesture else {
            return;
        };
        // The layers whose bars (Layers) or clips (Tracks) the box touches;
        // the session takes them only when it covers no keys (v1's rule).
        let plan = session.update(keys, if in_graph { vec![] } else { covered.clips });
        self.queue.retain(|j| !matches!(j, Job::Apply(_)));
        self.queue.push_back(Job::Apply(plan));
        self.pump(cx);
    }
    fn on_up(&mut self, cx: &mut Context<Self>) -> bool {
        self.row_scroll = None;
        self.transform = None;
        self.readout = None;
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Idle => return false,
            Gesture::ParentPick {
                ids, target, moved, ..
            } => {
                if moved {
                    if let Some(target) = target {
                        self.queue.push_back(Job::Apply(self.model.parent(
                            &ids,
                            Some(target),
                            self.delegate.time(cx),
                        )));
                    }
                } else {
                    self.delegate
                        .run("timeline.parentMenu", vec![serde_json::json!(ids)], cx);
                }
            }
            // A click on a name stays the row's own (it selects).
            Gesture::LayerPress { .. } => return false,
            // One undo step ("Move layers"), through the model's drop.
            Gesture::Reorder { moving, drop, .. } => {
                if let Some(drop) = drop {
                    let time = self.delegate.time(cx);
                    match self.model.drop_layers(
                        &moving,
                        &drop.target,
                        drop.mode,
                        time,
                        pm_kernel::Origin::Interface,
                    ) {
                        Ok(plan) => self.queue.push_back(Job::Apply(plan)),
                        Err(error) => self.delegate.report(error.to_string(), cx),
                    }
                }
            }
            Gesture::Pressed(press) => {
                if let Some(Spec::Property { layer, path }) = press.spec {
                    self.delegate.run(
                        "timeline.propertyEdit",
                        vec![serde_json::json!({"type":"propertyEdit","layer":layer,"path":path})],
                        cx,
                    );
                }
                if let Some(job) = press.release {
                    self.queue.push_back(job);
                }
            }
            Gesture::Dragging { .. } => {
                self.transform = None;
                self.graph_axis = false;
                self.readout = None;
                self.queue.push_back(Job::Finish(true))
            }
            Gesture::Marquee {
                session,
                origin,
                at,
            } => {
                let delta = at - origin;
                let moved = f32::from(delta.x).abs().max(f32::from(delta.y).abs())
                    >= self.tokens.metrics.drag_threshold;
                if moved {
                    self.queue.push_back(Job::Apply(session.finish()));
                } else {
                    self.queue.push_back(Job::Apply(Plan {
                        calls: vec![SelectionCall::Clear.into()],
                        ..Default::default()
                    }));
                }
            }
            Gesture::Column
            | Gesture::Pan { .. }
            | Gesture::Zoom
            | Gesture::Navigate { .. }
            | Gesture::Rows { .. } => {
                self.lit = None;
                self.changed(true, cx)
            }
            Gesture::Scrub(_) => {}
        }
        cx.notify();
        self.pump(cx);
        true
    }
    fn on_wheel(&mut self, event: &ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(window.line_height());
        let (dx, dy) = (f32::from(delta.x) as f64, f32::from(delta.y) as f64);
        let (x, y) = self.local(event.position);
        // ⌘, ⌥ or Control with the wheel zooms the time (v1, as a trackpad
        // pinch reads).
        if event.modifiers.alt || event.modifiers.platform || event.modifiers.control {
            if x >= self.view.layer_column {
                self.zoom_at(x, (dy * WHEEL_ZOOM).exp(), cx);
            }
            return;
        }
        // v1: over the graph the wheel pans its values, and sideways travel
        // still scrolls time.
        if !event.modifiers.shift
            && dy.abs() >= dx.abs()
            && self.covered(x, y)
            && matches!(self.gesture, Gesture::Idle)
            && let (Some(g), Some(area)) = (self.graph.clone(), self.graph_area())
        {
            let shift = dy * g.value_per_point(area.height);
            let (min, max) = (g.min + shift, g.max + shift);
            self.graph_view = Some((g.view.clone(), min, max));
            self.graph = Some(Arc::new((*g).clone().with_range(min, max)));
            self.view.scroll_time -= dx / self.view.pps;
            self.clamp_time();
            self.changed_while_hot(cx);
            return;
        }
        let horizontal = dx.abs() > dy.abs() || event.modifiers.shift;
        if horizontal {
            let travel = if dx.abs() > dy.abs() { dx } else { dy };
            self.view.scroll_time -= travel / self.view.pps;
            self.clamp_time();
        } else {
            self.view.scroll_y = (self.view.scroll_y - dy).max(0.);
            self.clamp_scroll();
        }
        self.changed_while_hot(cx);
    }
    fn on_context(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = self.local(event.position);
        let Some(frame) = self.frame.clone() else {
            return;
        };
        let ruler = self.tokens.metrics.ruler as f64;
        // v1: a property row's name offers its lane's menu (Add Keyframe at
        // the playhead, Paste, Remove All, Separate Dimensions).
        let mut column_track = None;
        if x < self.view.layer_column {
            let row = self
                .scene
                .as_ref()
                .and_then(|s| s.row_at(y - ruler + self.view.scroll_y))
                .and_then(|r| pm_panel_timeline::v4::parse_row(&r.id).ok());
            match row {
                Some(RowId::Layer(id)) => {
                    self.layer_menu(id, window, cx);
                    return;
                }
                Some(RowId::Property { layer_id, channel }) => {
                    column_track = Some(MenuContext::Track {
                        layer: layer_id,
                        channel,
                    });
                }
                _ => return,
            }
        }
        let time = if column_track.is_some() {
            self.delegate.time(cx)
        } else {
            self.time_at(x)
        };
        let at = |kind: &str, layer: Option<&str>| {
            vec![(
                "timeline:context",
                serde_json::json!({"kind":kind,"layerId":layer,"time":time}),
            )]
        };
        let context = if let Some(track) = column_track {
            track
        } else if y < ruler {
            match frame.hit_op("marker:", x, y, 2.) {
                Some(op) => MenuContext::Marker {
                    id: op.id["marker:".len()..].to_owned(),
                },
                None => MenuContext::Ruler {
                    time: self.time_at(x),
                },
            }
        } else if self.graph_region().is_some_and(|a| y >= a.y) {
            let key = self.graph_ops(None).and_then(|ops| {
                ops.iter()
                    .filter_map(|o| {
                        let id = o.id.strip_prefix("graph-key:")?;
                        let (cx, cy) = (
                            o.bounds.x + o.bounds.width / 2.,
                            o.bounds.y + o.bounds.height / 2.,
                        );
                        Some(((cx - x).hypot(cy - y), id.to_owned()))
                    })
                    .filter(|(d, _)| *d < graph_render::POINT_HIT)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, id)| id)
            });
            match key {
                Some(id) => MenuContext::Keys { ids: vec![id] },
                None => MenuContext::Graph {
                    series: self.graph.as_ref().map_or(0, |_| 1),
                },
            }
        } else if let Some((_, key)) = frame.hit_key(x, y, 3.) {
            MenuContext::Keys { ids: vec![key] }
        } else if let Some((layer, _)) = frame.hit_clip(x, y) {
            let layer = layer.to_owned();
            self.layer_menu(layer, window, cx);
            return;
        } else {
            let row = self
                .scene
                .as_ref()
                .and_then(|s| s.row_at(y - ruler + self.view.scroll_y))
                .and_then(|r| pm_panel_timeline::v4::parse_row(&r.id).ok());
            match row {
                Some(RowId::Property { layer_id, channel }) => MenuContext::Track {
                    layer: layer_id,
                    channel,
                },
                _ => MenuContext::Empty,
            }
        };
        // Main's contexts: a row's kind, `layerId` only on a layer row (with
        // `layer:context` too), `empty` on the ruler and between rows.
        let key_row = frame
            .hit_key(x, y, 3.)
            .and_then(|(row, _)| pm_panel_timeline::v4::parse_row(&row).ok());
        let locations = match (&context, key_row) {
            (MenuContext::Keys { .. }, Some(RowId::Layer(id))) => {
                let mut locations = vec![("layer:context", serde_json::json!({ "layerId": id }))];
                locations.extend(at("layer", Some(&id)));
                locations
            }
            (MenuContext::Keys { .. } | MenuContext::Track { .. }, _) => at("prop", None),
            (MenuContext::Graph { .. }, _) => at("graph", None),
            _ => at("empty", None),
        };
        let items = self.model.menu(&context);
        let ids: Vec<&'static str> = items.iter().map(|i| i.id).collect();
        let mut rows: Vec<serde_json::Value> = items
            .iter()
            .map(|i| serde_json::json!({"id":i.id,"label":pm_panel_timeline::menu_label(i.id),"disabled":!i.enabled,"checked":i.checked}))
            .collect();
        let preset_index = ids.len();
        if matches!(context, MenuContext::Track { .. }) {
            rows.push(
                serde_json::json!({"id":"saveAnimationPreset","label":"Save Animation Preset…"}),
            );
        }
        let camera_index = ids.len() + 1;
        if matches!(context, MenuContext::Empty) {
            rows.push(empty_new_menu());
        }
        let weak = cx.entity().downgrade();
        self.delegate.clone().extension_menu(
            locations,
            rows,
            window,
            cx,
            Box::new(move |index, cx| {
                if index == Some(preset_index) && matches!(context, MenuContext::Track { .. }) {
                    let _ = weak.update(cx, |s, cx| {
                        s.delegate.run(
                            "saveAnimationPreset",
                            match &context {
                                MenuContext::Track { layer, channel } => {
                                    vec![serde_json::json!({"layer":layer,"properties":[channel]})]
                                }
                                _ => vec![],
                            },
                            cx,
                        )
                    });
                    return;
                }
                if index == Some(camera_index) && matches!(context, MenuContext::Empty) {
                    let _ = weak.update(cx, |s, cx| s.delegate.run(EMPTY_CAMERA_COMMAND, vec![], cx));
                    return;
                }
                let Some(action) = index.and_then(|i| ids.get(i)) else {
                    return;
                };
                let _ = weak.update(cx, |s, cx| {
                    let time = s.delegate.time(cx);
                    s.queue.push_back(Job::Menu {
                        context,
                        action: (*action).to_owned(),
                        time,
                    });
                    s.pump(cx);
                });
            }),
        );
    }
    /// The shared layer menu runs ordinary commands on the row's selection.
    fn layer_menu(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.model.selection.contains(&id) {
            self.queue
                .push_back(Job::SelectLayer(id.clone(), Mods::default()));
            self.pump(cx);
        }
        let delegate = self.delegate.clone();
        let mut layers: Vec<String> = self.model.selection.layers().to_vec();
        if !layers.contains(&id) {
            layers = vec![id.clone()];
        }
        let time = self.delegate.time(cx);
        let locations = vec![
            (
                "layer:context",
                serde_json::json!({"layerId": id, "layerIds": layers}),
            ),
            (
                "timeline:context",
                serde_json::json!({"kind":"layer","layerId":id,"time":time}),
            ),
        ];
        let rows = delegate.layer_menu_rows(&id, cx);
        self.delegate.clone().extension_menu(
            locations,
            rows.clone(),
            window,
            cx,
            Box::new(move |index, cx| {
                if let Some((command, args)) =
                    index.and_then(|i| crate::layer_commands::layer_menu_choice(&rows, i))
                {
                    delegate.run(&command, args, cx);
                }
            }),
        );
    }
    /// The skimmer follows the pointer over the time area (ruler, tracks
    /// and clips); over the layer column or with skimming off it hides.
    fn skim_to(&mut self, x: f64, cx: &mut Context<Self>) {
        let width = f32::from(self.bounds.size.width) as f64;
        let at = (self.delegate.skimming(cx) && x >= self.view.layer_column && x <= width)
            .then(|| frame_time(&self.model, self.time_at(x)).clamp(0., self.model.index.duration));
        self.set_skimmer(at, cx);
    }
    fn set_skimmer(&mut self, at: Option<f64>, cx: &mut Context<Self>) {
        if at.is_none() && self.hover_clip.take().is_some() {
            cx.notify();
        }
        if at != self.skimmer {
            self.skimmer = at;
            self.delegate.skim(at, cx);
            cx.notify();
        }
    }
    /// Development builds' review sweep: the pointer over `seconds` on the
    /// ruler (`None`: it left), through the hover handler.
    #[cfg(debug_assertions)]
    pub fn review_hover(&mut self, seconds: Option<f64>, cx: &mut Context<Self>) {
        match seconds {
            Some(t) => {
                let x = self.view.layer_column + (t - self.view.scroll_time) * self.view.pps;
                let at =
                    self.bounds.origin + point(px(x as f32), px(self.tokens.metrics.ruler / 2.));
                self.hover(at, cx);
            }
            None => self.set_skimmer(None, cx),
        }
    }
    /// View ▸ Skimming changed, or the controller dropped the skimmer
    /// (Space with the pointer elsewhere): hide it unless the pointer still
    /// skims here.
    pub fn skimming_changed(&mut self, cx: &mut Context<Self>) {
        if !self.delegate.skimming(cx) || !self.delegate.skimmer_shown(cx) {
            self.set_skimmer(None, cx);
        }
    }
    #[cfg(test)]
    pub fn skimmer(&self) -> Option<f64> {
        self.skimmer
    }
    /// Whether the pointer is over the time area right now (Space plays
    /// from the skimmer only then): the window is active and the pointer's
    /// last position in it is inside this element.
    pub fn pointer_over(&self, window: &Window) -> bool {
        self.skimmer.is_some()
            && window.is_window_active()
            && window.is_window_hovered()
            && self.bounds.contains(&window.mouse_position())
    }
    /// v1's drag readout: one key reads where it is (value and time); a
    /// group how far it travelled. The Speed Graph reads time alone.
    fn graph_readout(&self, sample: &DragSample) -> String {
        let fps = self.model.index.fps;
        let speed = self.graph.as_ref().is_some_and(|g| g.speed);
        let keys = self.model.selection.keys();
        if let [id] = keys
            && let Some((layer, _, key)) = self.model.index.key(id)
        {
            let time = layer.from + key.time + frame(sample.time_delta, fps);
            let tc = crate::captions_panel::timecode(time, fps);
            return match key.value.as_f64().filter(|_| !speed) {
                Some(v) => format!(
                    "{}  ·  {tc}",
                    graph_render::value_text(v + sample.value_delta, 2)
                ),
                None => tc,
            };
        }
        let d = frame(sample.time_delta, fps);
        let time = format!(
            "{}{}",
            if d >= 0. { "+" } else { "\u{2212}" },
            crate::captions_panel::timecode(d.abs(), fps)
        );
        if speed {
            time
        } else {
            format!(
                "\u{394} {}  ·  {time}",
                graph_render::value_text(sample.value_delta, 2)
            )
        }
    }
    /// Whether the graph covers `(x, y)` (surface points): the layer bars
    /// under it are hidden there, so nothing may hover, trim or press them.
    fn covered(&self, x: f64, y: f64) -> bool {
        self.graph_region()
            .is_some_and(|r| x >= r.x && y >= r.y && y <= r.y + r.height)
    }
    /// The layer bar at `(x, y)` and the edge under it, where a bar shows.
    fn clip_at(&self, x: f64, y: f64) -> Option<(String, Option<Edge>)> {
        if self.covered(x, y) {
            return None;
        }
        self.frame
            .as_ref()
            .and_then(|f| f.hit_clip(x, y))
            .map(|(id, edge)| (id.to_owned(), edge))
    }
    /// The layer bar hovered now (tests).
    pub fn hovered_clip(&self) -> Option<String> {
        self.hover_clip.as_ref().map(|(id, _)| id.clone())
    }
    fn hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (x, y) = self.local(position);
        self.skim_to(x, cx);
        let hovered = self.clip_at(x, y).map(|(id, _)| (id, x));
        // The grip only changes side at the strip's middle: repaint when the
        // clip or the side changes, not on every pointer move.
        let side = |h: &Option<(String, f64)>| {
            h.as_ref().map(|(id, x)| {
                let mid = self.frame.as_ref().and_then(|f| {
                    f.ops
                        .iter()
                        .find(|o| o.id.strip_prefix("clip:") == Some(id))
                        .map(|o| o.bounds.x + o.bounds.width / 2.)
                });
                (id.clone(), mid.is_some_and(|mid| *x < mid))
            })
        };
        if side(&hovered) != side(&self.hover_clip) {
            cx.notify();
        }
        self.hover_clip = hovered;
        let m = self.tokens.metrics;
        let cursor = if (x - self.view.layer_column).abs() <= m.hit as f64 / 4.
            && y >= m.ruler as f64
        {
            CursorStyle::ResizeColumn
        } else if self.work_handle_at(x, y).is_some() {
            CursorStyle::ResizeLeftRight
        } else if x >= self.view.layer_column && (0. ..m.ruler as f64).contains(&y) {
            CursorStyle::ResizeLeftRight
        } else if self.frame.as_ref().is_some_and(|f| f.hit_layer_key(x,y,((m.hit-m.keyframe) as f64/2.).max(0.)).is_some()) {
            CursorStyle::PointingHand
        } else if let Some((id, Some(_))) = self.clip_at(x, y)
                // A locked layer's ends offer no trim.
                && !self.frame.as_ref().is_some_and(|f| {
                    f.ops.iter().any(|o| {
                        o.id.strip_prefix("clip:") == Some(id.as_str()) && o.detail["cursor"] == "notAllowed"
                    })
                })
        {
            CursorStyle::ResizeLeftRight
        } else if self.clip_at(x,y).is_some_and(|(id,_)| !self.model.index.layer(&id).is_some_and(|l| l.effectively_locked)) {
            CursorStyle::OpenHand
        } else {
            CursorStyle::Arrow
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            cx.notify();
        }
        let text = self.frame.as_ref().and_then(|f| f.tooltip_at(x, y));
        let shown = self.tip.as_ref().map(|(t, _)| t.to_string());
        if text != shown {
            if self.tip.take().is_some() {
                cx.notify();
            }
            self.tip_wait = text.map(|text| {
                let delay = std::time::Duration::from_secs_f64(self.tokens.motion.tooltip_delay);
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(delay).await;
                    let _ = this.update(cx, |s, cx| {
                        s.tip = Some((text.into(), (x, y)));
                        cx.notify();
                    });
                })
            });
        }
    }
    #[cfg(test)]
    pub fn graph_ops_for_test(&self) -> Vec<Op> {
        self.graph_ops(None).unwrap_or_default()
    }
    /// Points a second, the time at the time area's left edge, and the
    /// time area's width less the room Fit Selected Curves leaves.
    #[cfg(test)]
    pub fn time_view(&self) -> (f64, f64, f64) {
        (
            self.view.pps,
            self.view.scroll_time,
            f32::from(self.bounds.size.width) as f64 - self.view.layer_column - 48.,
        )
    }
    #[cfg(test)]
    pub fn graph(&self) -> Option<Arc<Graph>> {
        self.graph.clone()
    }
    #[cfg(test)]
    pub fn scene(&self) -> Option<Arc<Scene>> {
        self.scene.clone()
    }
    #[cfg(test)]
    pub fn frame(&self) -> Option<Arc<Frame>> {
        self.frame.clone()
    }
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        !self.busy && self.queue.is_empty() && self.client.is_some()
    }
    #[cfg(test)]
    pub fn pps(&self) -> f64 {
        self.view.pps
    }
    #[cfg(test)]
    pub fn scroll_time(&self) -> f64 {
        self.view.scroll_time
    }
    #[cfg(test)]
    pub fn scroll_y(&self) -> f64 {
        self.view.scroll_y
    }
    #[cfg(test)]
    pub fn layer_column(&self) -> f64 {
        self.view.layer_column
    }
    #[cfg(test)]
    /// The pointer the timeline shows (tests).
    pub fn cursor_style(&self) -> CursorStyle {
        self.cursor
    }
    pub fn model(&self) -> Arc<TimelineModel> {
        self.model.clone()
    }
}
/// The graph to show after a rebuild. A key or handle drag maps the pointer
/// through the value range, so from the press until release the range on
/// screen is held: selecting the key, its handles appearing and every step
/// of the drag leave the curve under the pointer. Between gestures the range
/// on screen is also kept while the rebuilt curve still fits it well
/// (`Graph::fits_in`), so a release, an undo or a nudge doesn't rescale the
/// graph; a curve that leaves the range, or shrinks to under half of it, or
/// another property or graph mode, refits.
fn steady_graph(
    gesture: &Gesture,
    shown: Option<&Graph>,
    next: Option<Arc<Graph>>,
) -> Option<Arc<Graph>> {
    let (Some(shown), Some(next)) = (shown, next.as_deref()) else {
        return next;
    };
    if shown.view != next.view {
        return Some(Arc::new(next.clone()));
    }
    let held = matches!(
        gesture,
        Gesture::Dragging { .. }
            | Gesture::Pressed(Press {
                spec: Some(Spec::GraphKeys | Spec::Handle { .. } | Spec::Transform { .. }),
                ..
            })
    );
    if held || next.fits_in(shown.min, shown.max) {
        Some(Arc::new(next.clone().with_range(shown.min, shown.max)))
    } else {
        Some(Arc::new(next.clone()))
    }
}
fn frame_time(model: &TimelineModel, time: f64) -> f64 {
    frame(time, model.index.fps)
}
async fn run(client: &mut TimelineClient, job: Job) -> Result<Vec<HostEffect>, String> {
    let text = |e: pm_panel_timeline::ClientError| e.to_string();
    match job {
        Job::Sync => {
            client.sync().map_err(text)?;
            client.adopt_view().map_err(text)?;
            Ok(vec![])
        }
        Job::SelectLayer(id, mods) => {
            let calls = if client.model.mode == TimelineMode::Tracks {
                client.model.select_track_layer(&id, mods).map(|r| r.0)
            } else {
                client.model.select_layer(&id, mods)
            }
            .map_err(|e| e.to_string())?;
            client
                .dispatch(Plan {
                    calls,
                    ..Default::default()
                })
                .await
                .map_err(text)
        }
        Job::SelectKey {
            layer,
            channel,
            key,
            additive,
            dragging,
        } => {
            let calls = client
                .model
                .select_keys(&layer, &channel, &[key], additive, dragging)
                .map_err(|e| e.to_string())?;
            client
                .dispatch(Plan {
                    calls,
                    ..Default::default()
                })
                .await
                .map_err(text)
        }
        Job::Begin(spec, time) => {
            let kind = drag_kind(&client.model, spec, time).map_err(|e| e.to_string())?;
            if matches!(&kind, DragKind::Layers { .. })
                && client.model.mode == TimelineMode::Layers
                && client.model.graph_display == GraphDisplay::DopeSheet
            {
                client.begin_preview_drag(kind).map_err(text)?;
            } else {
                client.begin_drag(kind).await.map_err(text)?;
            }
            Ok(vec![])
        }
        Job::Sample(sample) => {
            if client.active_drag().is_some() {
                client.drag(sample).await.map_err(text)?;
            }
            Ok(vec![])
        }
        Job::Finish(commit) => {
            if let Some(drag) = client.active_drag() {
                let copies = drag.copies().map(<[String]>::to_vec);
                client.finish_drag(commit).await.map_err(text)?;
                // An Option-drag leaves its copies selected.
                if let (true, Some(ids)) = (commit, copies) {
                    client
                        .dispatch(Plan {
                            calls: vec![SelectionCall::Layers { ids, add: false }.into()],
                            ..Default::default()
                        })
                        .await
                        .map_err(text)?;
                }
            }
            Ok(vec![])
        }
        Job::Apply(plan) => client.dispatch(plan).await.map_err(text),
        Job::PasteKeys(tracks, time) => {
            let mut effects = vec![];
            for track in tracks {
                let plan = client
                    .model
                    .paste_keys(&track.layer_id, &track.channel, time, &track.keys)
                    .map_err(|e| e.to_string())?;
                effects.extend(client.dispatch(plan).await.map_err(text)?);
            }
            Ok(effects)
        }
        Job::Command {
            id,
            time,
            frames,
            shift,
        } => {
            let persist =
                id.starts_with("timeline.") || id.starts_with("graph") || id == "toggleLayerStrips";
            let plan = client
                .model
                .command(
                    &id,
                    time,
                    frames,
                    shift,
                    millis(),
                    &PropertyFacts::default(),
                )
                .map_err(|e| e.to_string())?;
            let effects = client.dispatch(plan).await.map_err(text)?;
            if persist {
                client.publish_view().await.map_err(text)?;
            }
            Ok(effects)
        }
        Job::Menu {
            context,
            action,
            time,
        } => {
            let persist = action == "mode.toggle" || action.starts_with("graph.");
            let plan = client
                .model
                .menu_action(&context, &action, time)
                .map_err(|e| e.to_string())?;
            let effects = client.dispatch(plan).await.map_err(text)?;
            if persist {
                client.publish_view().await.map_err(text)?;
            }
            Ok(effects)
        }
        Job::Reveal { layer, keys } => {
            client
                .model
                .reveal_paths(&layer, &keys)
                .map_err(|e| e.to_string())?;
            client.publish_view().await.map_err(text)?;
            Ok(vec![])
        }
        Job::ShowGraph {
            layer,
            channel,
            keys,
        } => {
            let plan = client
                .model
                .show_graph(&layer, &channel, &keys)
                .map_err(|e| e.to_string())?;
            let effects = client.dispatch(plan).await.map_err(text)?;
            client.publish_view().await.map_err(text)?;
            Ok(effects)
        }
        Job::Persist(view) => {
            use pm_kernel::state::{StateOp, StateTransaction};
            let state = client.kernel.state();
            let mut ui = state.ui.get(&client.project).cloned().unwrap_or_default();
            let mut timeline = ui.timeline.as_object().cloned().unwrap_or_default();
            timeline.insert("nativePps", pm_model::Value::from(view.pps));
            timeline.insert("nativeScrollTime", pm_model::Value::from(view.scroll_time));
            timeline.insert("nativeScrollY", pm_model::Value::from(view.scroll_y));
            if view.column_chosen {
                timeline.insert(
                    "nativeLayerColumn",
                    pm_model::Value::from(view.layer_column),
                );
            } else {
                timeline.remove("nativeLayerColumn");
            }
            ui.timeline = pm_model::Value::Object(timeline);
            let pending = client.kernel.state_transaction(StateTransaction {
                ops: vec![StateOp::Ui {
                    project: client.project.clone(),
                    next: ui,
                }],
                ..Default::default()
            });
            pending
                .await
                .ok_or("Kernel closed")?
                .map_err(|e| format!("{e:?}"))?;
            Ok(vec![])
        }
    }
}
fn millis() -> u64 {
    static CLOCK: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    CLOCK
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}
fn drag_kind(model: &TimelineModel, spec: Spec, time: f64) -> Result<DragKind, String> {
    use pm_kernel::commands::pro_edit::{Edge as ProEdge, ProEditCommand as Pro, TrackTarget};
    let ids = model.selection.layers().to_vec();
    let keys = model.expand_scale_keys(model.selection.keys());
    let primary = || ids.first().cloned().ok_or("Select a clip first");
    let pro_edge = |edge: Edge| match edge {
        Edge::In => ProEdge::In,
        Edge::Out => ProEdge::Out,
    };
    let lanes = pm_panel_timeline::tracks::layout(&model.index, &model.reveal);
    let neighbours = |id: &str| {
        let item = lanes.item(id).ok_or("Clip is not on a track")?;
        let same = |other: &&pm_panel_timeline::tracks::Item| {
            other.id != id && other.area == item.area && other.lane == item.lane
        };
        let before = lanes
            .items
            .iter()
            .filter(same)
            .find(|o| (o.from + o.duration - item.from).abs() < 1e-6)
            .map(|o| o.id.clone());
        let after = lanes
            .items
            .iter()
            .filter(same)
            .find(|o| (item.from + item.duration - o.from).abs() < 1e-6)
            .map(|o| o.id.clone());
        Ok::<_, &str>((before, after))
    };
    // In Layers mode every layer is its own row: the tools run the
    // layer-level edits (all layer kinds; what overlaps stays), as the keys do.
    let layered = model.mode == TimelineMode::Layers;
    let edge_time = |id: &str, edge: Edge| {
        model.index.layer(id).map(|l| match edge {
            Edge::In => l.from,
            Edge::Out => l.from + l.duration,
        })
    };
    let timing_edge = |edge: Edge| match edge {
        Edge::In => pm_kernel::commands::timing::TimeEdge::In,
        Edge::Out => pm_kernel::commands::timing::TimeEdge::Out,
    };
    use pm_kernel::TimingCommand as Timing;
    if layered && matches!(spec, Spec::Ripple(_) | Spec::Roll(_) | Spec::Slide) {
        let id = primary()?;
        return Ok(DragKind::Timing(match spec {
            Spec::Ripple(edge) => Timing::RippleTrim {
                layer_ids: vec![id.clone()],
                edge: timing_edge(edge),
                time: edge_time(&id, edge).ok_or("Unknown clip")?,
            },
            Spec::Roll(edge) => Timing::Roll {
                layer_id: id.clone(),
                edge: timing_edge(edge),
                time: edge_time(&id, edge).ok_or("Unknown clip")?,
            },
            _ => Timing::Slide {
                layer_id: id,
                delta: 0.,
            },
        }));
    }
    Ok(match spec {
        Spec::Layers => DragKind::Layers { ids },
        Spec::Duplicate => {
            // A selected group brings its members: only the tops are copied.
            let inside = |id: &str| {
                let mut at = model.index.layer(id).and_then(|l| l.group.clone());
                for _ in 0..256 {
                    let Some(group) = at else { return false };
                    if ids.contains(&group) {
                        return true;
                    }
                    at = model.index.layer(&group).and_then(|l| l.group.clone());
                }
                false
            };
            let tops: Vec<String> = ids.iter().filter(|id| !inside(id)).cloned().collect();
            if tops.is_empty() {
                return Err("Select a layer first".into());
            }
            DragKind::DuplicateLayers {
                new_ids: tops.iter().map(|_| pm_kernel::new_id("L")).collect(),
                ids: tops,
            }
        }
        Spec::Trim(Edge::In) => DragKind::TrimIn { ids },
        Spec::Trim(Edge::Out) => DragKind::TrimOut { ids },
        Spec::Offset => DragKind::QuickOffsetLayers { ids },
        Spec::Keys => DragKind::Keys { ids: keys },
        Spec::TextStagger { band, end } => DragKind::TextStagger { band, end, time },
        Spec::QuickOffsetKeys => {
            let groups: Vec<Vec<String>> = model
                .index
                .layers
                .iter()
                .filter_map(|layer| {
                    let ids: Vec<_> = keys
                        .iter()
                        .filter(|id| {
                            model
                                .index
                                .key(id)
                                .is_some_and(|(l, _, _)| l.id == layer.id)
                        })
                        .cloned()
                        .collect();
                    (!ids.is_empty()).then_some(ids)
                })
                .collect();
            if groups.len() < 2 {
                DragKind::Keys { ids: keys }
            } else {
                DragKind::QuickOffsetKeys { groups, time }
            }
        }
        Spec::StretchKeys { grabbed } => {
            // v1's keyframeStretchAnchor: two or more keys apart, grabbed at
            // the first or last; anything else moves them.
            let times: Vec<f64> = keys
                .iter()
                .filter_map(|id| model.index.key(id).map(|(l, _, k)| l.from + k.time))
                .collect();
            let first = times.iter().copied().fold(f64::INFINITY, f64::min);
            let last = times.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let anchor = (times.len() >= 2 && last - first >= 1e-9)
                .then(|| {
                    if (grabbed - last).abs() < 1e-6 {
                        Some(first)
                    } else if (grabbed - first).abs() < 1e-6 {
                        Some(last)
                    } else {
                        None
                    }
                })
                .flatten();
            match anchor {
                Some(anchor_time) => DragKind::StretchKeys {
                    ids: keys,
                    anchor_time,
                    grabbed_time: grabbed,
                },
                None => DragKind::Keys { ids: keys },
            }
        }
        Spec::GraphKeys => DragKind::GraphKeys { ids: keys },
        Spec::Transform {
            anchor_time,
            anchor_value,
        } => DragKind::TransformKeys {
            ids: keys,
            anchor_time,
            anchor_value,
        },
        Spec::Handle { key, side } => DragKind::Handle { id: key, side },
        Spec::Property { layer, path } => {
            let paths: Vec<_> = model
                .track_channels(&layer, &path)
                .iter()
                .map(|c| c.path.clone())
                .collect();
            DragKind::Property {
                targets: model
                    .property_targets(&layer, &paths)
                    .map_err(|e| e.to_string())?,
                time,
            }
        }
        Spec::WorkIn => DragKind::WorkIn,
        Spec::WorkOut => DragKind::WorkOut,
        Spec::WorkMove => DragKind::WorkMove,
        Spec::Slip => DragKind::ProEdit(Pro::Slip {
            layer_ids: ids,
            delta: 0.,
            linked: true,
        }),
        Spec::Ripple(edge) => {
            let id = primary()?;
            let layer = model.index.layer(&id).ok_or("Unknown clip")?;
            DragKind::ProEdit(Pro::RippleTrim {
                layer_id: id.clone(),
                edge: pro_edge(edge),
                delta: 0.,
                tracks: vec![TrackTarget {
                    track: layer.preferred_track as i32,
                    group_id: layer.group.clone(),
                }],
                linked: true,
            })
        }
        Spec::Roll(edge) => {
            let id = primary()?;
            let (before, after) = neighbours(&id)?;
            let (left, right) = match edge {
                Edge::In => (before.ok_or("No clip before this edit")?, id),
                Edge::Out => (id, after.ok_or("No clip after this edit")?),
            };
            DragKind::ProEdit(Pro::Roll {
                left_id: left,
                right_id: right,
                delta: 0.,
                linked: true,
            })
        }
        Spec::Slide => {
            let id = primary()?;
            let (before, after) = neighbours(&id)?;
            DragKind::ProEdit(Pro::Slide {
                previous_id: before.ok_or("No clip before this one")?,
                layer_id: id,
                next_id: after.ok_or("No clip after this one")?,
                delta: 0.,
                linked: true,
            })
        }
    })
}
struct Presenter {
    offline: Arc<BTreeSet<String>>,
    sources: Sources,
    wanted: RefCell<Vec<crate::waveforms::Key>>,
    published: Arc<Published>,
}
impl ModelPresenter for Presenter {
    fn accessibility_label(&self, row: &RowId) -> String {
        match row {
            RowId::Layer(id) => id.clone(),
            RowId::Property { channel, .. } => channel.clone(),
            RowId::Effect { effect, .. } => effect.clone(),
        }
    }
    fn layer_label(&self, layer: &pm_panel_timeline::Layer) -> Label {
        Label::for_layer(&layer.kind, layer.definition.as_deref())
    }
    fn media(&self, layer: &pm_panel_timeline::Layer) -> Media {
        if layer
            .asset
            .as_ref()
            .is_some_and(|asset| self.offline.contains(asset))
        {
            Media::Offline
        } else {
            Media::Ready
        }
    }
    fn waveform(
        &self,
        layer: &pm_panel_timeline::Layer,
    ) -> Option<Option<pm_panel_timeline::render::Waveform>> {
        if layer.kind != "audio" {
            return None;
        }
        let asset = layer.asset.as_ref()?;
        let path = self.sources.get(asset)?;
        // The clip as its asset plays it: the streams heard, its timing.
        let record = self
            .published
            .snapshot
            .root()
            .and_then(|root| root.get("assets")?.get(asset));
        let key = crate::waveforms::Key::of(path.clone(), record);
        Some(match crate::waveforms::lookup(&key)? {
            Some((peaks, rate)) => Some(pm_panel_timeline::render::Waveform {
                peaks,
                rate,
                trim: layer.trim,
            }),
            None => {
                self.wanted.borrow_mut().push(key);
                None
            }
        })
    }
    fn cues(&self, layer: &pm_panel_timeline::Layer) -> Vec<pm_panel_timeline::render::CueMark> {
        if layer.kind != "captions" {
            return vec![];
        }
        let Some(root) = self.published.snapshot.root() else {
            return vec![];
        };
        let Some(node) = pm_kernel::model::find_layer(root, &layer.id).and_then(|at| {
            let comp = pm_kernel::model::comp_node(root, at.comp_id.as_deref())?;
            pm_kernel::model::layers_of(comp).nth(at.index)
        }) else {
            return vec![];
        };
        let Some(cues) = node.get("d").and_then(|d| d.get("cues")) else {
            return vec![];
        };
        let cues: Vec<pm_captions::CaptionCue> =
            serde_json::from_str(&pm_model::json::to_canonical_string(&cues.to_value()))
                .unwrap_or_default();
        cues.into_iter()
            .map(|c| pm_panel_timeline::render::CueMark {
                id: c.id,
                start: layer.from + c.start,
                end: layer.from + c.end,
                text: c.text,
            })
            .collect()
    }
    fn stagger_bands(
        &self,
        layer: &pm_panel_timeline::Layer,
        time: f64,
    ) -> Vec<pm_panel_timeline::stagger::Band> {
        if layer.kind != "text" {
            return vec![];
        }
        let Some(root) = self.published.snapshot.root() else {
            return vec![];
        };
        let Some(node) = pm_kernel::model::find_layer(root, &layer.id)
            .and_then(|at| pm_kernel::model::layer_at(root, &at))
        else {
            return vec![];
        };
        let Some(animators) = node.get("d").and_then(|d| d.get("animators")) else {
            return vec![];
        };
        let raw: serde_json::Value =
            serde_json::from_str(&pm_model::json::to_canonical_string(&animators.to_value()))
                .unwrap_or_default();
        if !raw.as_array().into_iter().flatten().any(|animator| {
            animator["enabled"] != false
                && animator["mode"] == "stagger"
                && animator["p"].get("duration").is_some()
        }) {
            return vec![];
        }
        crate::text_host::query(&self.published, |q| {
            let layout = q.text_layout(None, &layer.id, time)?;
            let lines: Vec<pm_model::JsString> = layout
                .get("lines")
                .and_then(pm_model::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|l| {
                    l.get("text")
                        .and_then(pm_model::Value::as_str)
                        .map(Into::into)
                })
                .collect();
            let mut bands = Vec::new();
            for animator in raw.as_array().into_iter().flatten() {
                if animator["enabled"] == false
                    || animator["mode"] != "stagger"
                    || animator["p"].get("duration").is_none()
                {
                    continue;
                }
                let Some(id) = animator["id"].as_str() else {
                    continue;
                };
                let unit = match animator["unit"].as_str() {
                    Some("words") => pm_eval::text_anim::UnitKind::Words,
                    Some("lines") => pm_eval::text_anim::UnitKind::Lines,
                    _ => pm_eval::text_anim::UnitKind::Characters,
                };
                let count = (pm_eval::text_anim::count_text_units(&lines, unit).max(1) as f64
                    + animator["unitOffset"].as_f64().unwrap_or(0.))
                .max(animator["unitTotal"].as_f64().unwrap_or(0.))
                .max(1.);
                let mut value = |key: &str| -> Result<f64, pm_kernel::QueryError> {
                    if animator["p"].get(key).is_none() {
                        return Ok(0.);
                    }
                    Ok(
                        q.evaluate(None, &layer.id, &format!("ta.{id}.{key}"), time)?
                            .as_f64()
                            .filter(|v| v.is_finite())
                            .unwrap_or(0.),
                    )
                };
                let (delay, duration_value, stagger) =
                    (value("delay")?, value("duration")?, value("stagger")?);
                let duration = duration_value.max(0.);
                let rank = (count - 1.).max(0.)
                    / if matches!(animator["order"].as_str(), Some("center" | "edges")) {
                        2.
                    } else {
                        1.
                    };
                let cascade = stagger.max(0.) * rank;
                let out = match animator["anchor"].as_str() {
                    Some("in") => false,
                    Some("out") => true,
                    _ => animator["direction"] == "out",
                };
                let legacy_duration = animator["legacyStartDuration"]
                    .as_f64()
                    .filter(|v| v.is_finite());
                let start = if out {
                    let offset = legacy_duration
                        .map(|duration| duration - delay - duration_value.max(0.) - cascade)
                        .unwrap_or(delay);
                    layer.duration - offset - duration - cascade
                } else {
                    delay
                };
                bands.push(pm_panel_timeline::stagger::Band {
                    layer: layer.id.clone(),
                    animator: id.into(),
                    delay,
                    duration_value,
                    stagger,
                    start,
                    duration,
                    cascade,
                    end: start + duration + cascade,
                    out,
                    legacy: legacy_duration.is_some(),
                });
            }
            Ok(bands)
        })
        .unwrap_or_default()
    }
    fn interpolation(&self, key: &pm_panel_timeline::Key) -> Interpolation {
        match key.stored_string("e") {
            Some("hold") => Interpolation::Hold,
            Some("linear") | None => Interpolation::Linear,
            _ => Interpolation::Ease,
        }
    }
    fn track_accessibility_label(
        &self,
        area: pm_panel_timeline::tracks::Area,
        lane: usize,
    ) -> Option<String> {
        Some(format!("{area:?} {}", lane + 1))
    }
}
/// The curves a key selection put in the graph stay there once every key is
/// deselected (v1's `graphShown`).
fn remember_shown(client: &mut TimelineClient, built: &Option<Built>) {
    if let Some(graph) = built
        .as_ref()
        .and_then(|b| b.graph.as_ref())
        .filter(|g| g.picked)
    {
        client.model.graph_shown = graph.series.clone();
    }
}
fn build(
    client: &TimelineClient,
    tokens: &Tokens,
    offline: Arc<BTreeSet<String>>,
    sources: Sources,
) -> Option<Built> {
    let published: Arc<Published> = client.kernel.published(&client.project)?;
    let presenter = Presenter {
        offline,
        sources,
        wanted: Default::default(),
        published: published.clone(),
    };
    let mut scene = Scene::from_model(&client.model, &presenter, tokens, 0.).ok()?;
    scene.bars = crate::render_frames::bars(client.model.index.comp_id.as_deref());
    let graph = (client.model.graph_display != GraphDisplay::DopeSheet)
        .then(|| {
            let mut mirror = QueryMirror::new(&published);
            let comp = client.model.index.comp_id.clone();
            graph_render::sample(&client.model, |layer, path, time| {
                mirror
                    .queries()
                    .evaluate(comp.as_deref(), layer, path, time)
                    .ok()?
                    .as_f64()
            })
        })
        .flatten();
    Some(Built {
        scene: Arc::new(scene),
        graph: graph.map(Arc::new),
        model: Arc::new(client.model.clone()),
        wanted: presenter.wanted.into_inner(),
    })
}
/// The frame the canvas prepared, and where, for the clips' accessible elements.
type Shown = Rc<RefCell<Option<(Point<Pixels>, Arc<Frame>)>>>;
impl Render for TimelineSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.activation.is_none() {
            self.activation = Some(cx.observe_window_activation(window, |s, window, cx| {
                if !window.is_window_active() {
                    s.set_skimmer(None, cx);
                }
            }));
        }
        let this = cx.entity().downgrade();
        let tokens = self.tokens.clone();
        let graph = self.graph_ops(Some(self.delegate.time(cx))).map(|mut ops| {
            if let (Some((text, x, y)), Some(region)) = (&self.readout, self.graph_region()) {
                ops.extend(graph_render::readout(text, *x, *y, region, &self.tokens));
            }
            ops
        });
        let region = self.graph_region();
        let column = self.view.layer_column;
        let decorations = self.decorations.clone();
        let hover_clip = self.hover_clip.clone();
        let skimmer = self.skimmer_at();
        let marquee = match &self.gesture {
            Gesture::Marquee { origin, at, .. } => Some(Bounds::from_corners(
                point(origin.x.min(at.x), origin.y.min(at.y)),
                point(origin.x.max(at.x), origin.y.max(at.y)),
            )),
            _ => None,
        };
        let weak = this.clone();
        let t = &self.tokens;
        // Media dragged over the timeline: where it would land, an
        // insertion line between layers and the frame under the pointer.
        if !cx.has_active_drag() {
            self.media_over = None;
            self.effect_over = None;
        }
        // An effect over a layer's row: the row it would join, outlined.
        let effect_row = self
            .effect_over
            .and_then(|position| self.layer_under(position))
            .map(|(_, y)| {
                div()
                    .absolute()
                    .left(px(0.))
                    .top(px(y as f32))
                    .w(self.bounds.size.width)
                    .h(px(self.tokens.metrics.track_row))
                    .border(px(self.tokens.metrics.border * 2.))
                    .border_color(rgb(self.tokens.colors.accent))
            });
        let landing = self.media_over.map(|position| {
            let (at, _, y) = self.media_slot(position);
            let x = self.view.layer_column + (at - self.view.scroll_time) * self.view.pps;
            let (width, height) = (self.bounds.size.width, self.bounds.size.height);
            let accent = rgb(self.tokens.colors.accent);
            [
                div()
                    .absolute()
                    .left(px(self.view.layer_column as f32))
                    .top(px(y as f32 - 1.))
                    .w(width - px(self.view.layer_column as f32))
                    .h(px(2.))
                    .bg(accent),
                div()
                    .absolute()
                    .left(px(x as f32))
                    .top(px(self.tokens.metrics.ruler))
                    .w(px(1.))
                    .h(height - px(self.tokens.metrics.ruler))
                    .bg(accent),
            ]
        });
        // v1's reorder cue: an accent line with a dot at its depth, or a
        // insertion line for a group, and what the drop will do.
        let reorder = match &self.gesture {
            Gesture::Reorder { drop: Some(d), .. } => Some(d.clone()),
            _ => None,
        }
        .map(|d| {
            use pm_panel_timeline::DropMode;
            let t = &self.tokens;
            let accent = t.colors.accent;
            let width = self.bounds.size.width;
            let left = px(d.left as f32);
            let cue = div()
                .absolute()
                .left(left)
                .top(px(d.line as f32 - 1.))
                .w(width - left)
                .h(px(2.))
                .bg(rgb(accent));
            let dot = Some(
                div()
                    .absolute()
                    .left(left - px(3.))
                    .top(px(d.line as f32 - 3.))
                    .size(px(6.))
                    .rounded_full()
                    .bg(rgb(accent)),
            );
            let verb = match d.mode {
                DropMode::Inside => "Move into",
                DropMode::Before => "Move before",
                DropMode::After => "Move after",
            };
            let label_top = (d.line as f32 - 24.)
                .max(t.metrics.ruler + 2.)
                .min(f32::from(self.bounds.size.height) - 24.);
            let label = div()
                .absolute()
                .left(px(self.view.layer_column as f32 + t.metrics.inset))
                .top(px(label_top))
                .px(px(t.metrics.gap))
                .py(px(t.metrics.border * 2.))
                .rounded(px(t.metrics.control_radius))
                .bg(rgb(accent))
                .text_size(px(t.typography.minimum))
                .text_color(rgb(0xffffff))
                .whitespace_nowrap()
                .child(format!("{verb} {}", d.name));
            [Some(cue), dot, Some(label)]
        });
        let tip = self.tip.clone().map(|(text, (x, y))| {
            // The shell's tooltip look: floating fill, separator edge, minimum type.
            div()
                .absolute()
                .left(px(x as f32 + t.metrics.inset))
                .top(px(y as f32 + t.metrics.inset))
                .px(px(t.metrics.gap))
                .py(px(t.metrics.border * 2.))
                .rounded(px(t.metrics.control_radius))
                .bg(rgb(t.colors.floating))
                .border(px(t.metrics.border))
                .border_color(rgb(t.colors.separator))
                .text_size(px(t.typography.minimum))
                .text_color(rgb(t.colors.text))
                .whitespace_nowrap()
                .child(text)
        });
        // The canvas prepaints before the list's children are written: it
        // leaves the frame it prepared here for the clips' accessible elements.
        let shown: Shown = Rc::default();
        let prepared = shown.clone();
        div()
            .id("native-timeline")
            .relative()
            .size_full()
            .cursor(self.cursor)
            .role(Role::ListBox)
            .aria_label(if region.is_some() && graph.is_none() {
                format!("Timeline tracks. {}", graph_render::EMPTY)
            } else {
                "Timeline tracks".into()
            })
            // Each clip is a selectable option VoiceOver and agents can find and
            // press (macOS: static text in a list, which AX walkers that skip
            // groups still report); a Press clicks its centre, the same path as
            // the pointer.
            .a11y_synthetic_children(move |children| {
                children.parent_node().set_multiselectable();
                let Some((origin, frame)) = shown.borrow_mut().take() else {
                    return;
                };
                for clip in frame.clip_nodes() {
                    // A bar the graph covers is hidden: no one can find or
                    // press it there.
                    if region.is_some_and(|r| clip.bounds.y >= r.y) {
                        continue;
                    }
                    let mut node = gpui::accesskit::Node::new(Role::ListBoxOption);
                    node.set_label(clip.label());
                    node.set_selected(clip.selected);
                    node.add_action(gpui::AccessibleAction::Click);
                    let b = clip.bounds;
                    let bounds = Bounds::new(
                        origin + point(px(b.x as f32), px(b.y as f32)),
                        size(px(b.width as f32), px(b.height as f32)),
                    );
                    let id = children.synthetic_node_id(&clip.id);
                    children.push_child_at(id, node, bounds);
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|s, event: &MouseDownEvent, _, cx| s.on_down(event, cx)),
            )
            // Finder files and Project items dropped here land at the time
            // and place in the stack under the pointer (v1.2.0's timeline).
            // GPUI tells every listener of a drag's moves: the line shows
            // only while the pointer is over the timeline.
            .on_drag_move(cx.listener(|s, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                s.media_moved(event.bounds, event.event.position, cx);
            }))
            .on_drag_move(cx.listener(
                |s, event: &DragMoveEvent<crate::v4_view::PanelDrag>, _, cx| {
                    let panel = event.drag(cx).panel.clone();
                    if panel == pm_builtins::PROJECT_PANEL {
                        s.media_moved(event.bounds, event.event.position, cx);
                    } else if panel == EFFECTS_PANEL {
                        s.effect_moved(event.bounds, event.event.position, cx);
                    }
                },
            ))
            .on_drop(cx.listener(|s, paths: &ExternalPaths, window, cx| {
                cx.stop_propagation();
                let what = Dropped::Files {
                    paths: paths.paths().to_vec(),
                    alt: window.modifiers().alt,
                };
                s.drop_media(what, window.mouse_position(), cx);
            }))
            .on_drop(cx.listener(|s, drag: &crate::v4_view::PanelDrag, window, cx| {
                if drag.panel == pm_builtins::PROJECT_PANEL {
                    cx.stop_propagation();
                    s.drop_media(Dropped::ProjectItems, window.mouse_position(), cx);
                } else if drag.panel == EFFECTS_PANEL
                    && let Some(definition) = &drag.item
                {
                    cx.stop_propagation();
                    s.effect_over = None;
                    let layer = s.layer_under(window.mouse_position()).map(|(id, _)| id);
                    s.delegate.drop_effect(
                        EffectDrop {
                            panel: drag.panel.clone(),
                            definition: definition.to_string(),
                            label: drag.label.to_string(),
                            layer,
                        },
                        cx,
                    );
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|s, event: &MouseDownEvent, window, cx| {
                    s.on_context(event, window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|s, event: &MouseDownEvent, _, _| {
                    s.gesture = Gesture::Pan { at: event.position };
                }),
            )
            .on_scroll_wheel(
                cx.listener(|s, event: &ScrollWheelEvent, window, cx| {
                    s.on_wheel(event, window, cx)
                }),
            )
            .on_pinch(cx.listener(|s, event: &PinchEvent, _, cx| {
                let (x, _) = s.local(event.position);
                s.zoom_at(x.max(s.view.layer_column), 1. + event.delta as f64, cx);
            }))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let frame = weak
                            .update(cx, |s, cx| {
                                s.body = bounds;
                                s.prepared(Self::above_bar(bounds, &s.tokens), cx)
                            })
                            .ok()
                            .flatten();
                        *prepared.borrow_mut() = frame.clone().map(|f| (bounds.origin, f));
                        frame
                    },
                    move |bounds, frame, window, cx| {
                        if let Some(frame) = frame {
                            // The outline row owns the keyframe toggle; the scene's
                            // copy would sit stale beneath it.
                            // Under a graph, the tracks stop at its top: the graph
                            // gets the plain material with a hairline over it.
                            let above = region.map(|r| {
                                Bounds::from_corners(
                                    bounds.origin,
                                    point(bounds.right(), bounds.origin.y + px(r.y as f32)),
                                )
                            });
                            let started = crate::diag::enabled().then(std::time::Instant::now);
                            let cull = Cull::new(bounds.size);
                            // The skimmer's timecode stands in the ruler: a time
                            // label it would cut through is left out, not
                            // half-covered (it is the time under the pointer).
                            let chip = skimmer.as_ref().map(|(x, code)| {
                                let m = tokens.metrics;
                                let run = TextRun {
                                    len: code.len(),
                                    font: window.text_style().font(),
                                    color: rgb_to_hsla(rgba((tokens.colors.text << 8) | 0xff)),
                                    background_color: None,
                                    underline: None,
                                    strikethrough: None,
                                    letter_spacing: None,
                                };
                                let text = window.text_system().shape_line(
                                    code.clone().into(),
                                    px(tokens.typography.minimum),
                                    &[run],
                                );
                                let left = x + (m.border * 2.) as f64;
                                let gutter = (m.gap + m.border * 2.) as f64;
                                (left, left + f32::from(text.width) as f64 + gutter)
                            });
                            for op in frame.ops.iter().filter(|op| !op.id.starts_with("toggle:")) {
                                if cull.skip(op) {
                                    continue;
                                }
                                if let Some((left, right)) = chip
                                    && op.id.starts_with("time:")
                                    && op.bounds.x < right
                                    && op.bounds.x + op.bounds.width > left
                                {
                                    // The label is centred in its step: its
                                    // text, not its box, decides.
                                    let text = op.text.clone().unwrap_or_default();
                                    let run = TextRun {
                                        len: text.len(),
                                        font: window.text_style().font(),
                                        color: rgb_to_hsla(rgba((tokens.colors.muted << 8) | 0xff)),
                                        background_color: None,
                                        underline: None,
                                        strikethrough: None,
                                        letter_spacing: None,
                                    };
                                    let half = f32::from(
                                        window
                                            .text_system()
                                            .shape_line(text.into(), px(op.font_size as f32), &[run])
                                            .width,
                                    ) as f64
                                        / 2.;
                                    let centre = op.bounds.x + op.bounds.width / 2.;
                                    if centre - half < right && centre + half > left {
                                        continue;
                                    }
                                }
                                // The time grid and the markers' lines run
                                // on through the graph: it reads the same time.
                                let through = op.id.starts_with("grid:")
                                    || op.id.starts_with("marker-guide:");
                                // A track op (a strip may start before the
                                // view) is placed by its clip, which starts
                                // at the layer column.
                                let left = op.clip.map_or(op.bounds.x, |c| c.x.max(op.bounds.x));
                                match above.filter(|_| !through) {
                                    // Wholly under the graph: not painted.
                                    Some(mask)
                                        if left >= column - 1.
                                            && bounds.origin.y + px(op.bounds.y as f32)
                                                >= mask.bottom() => {}
                                    Some(mask) if left >= column - 1. => window
                                        .with_content_mask(
                                            Some(ContentMask {
                                                bounds: mask,
                                                fade_out: Default::default(),
                                            }),
                                            |w| paint(op, bounds.origin, &tokens, w, cx),
                                        ),
                                    _ => paint(op, bounds.origin, &tokens, window, cx),
                                }
                            }
                            if let Some(started) = started {
                                let ms = started.elapsed().as_secs_f64() * 1000.;
                                if ms < 4. {
                                    // Every paint is counted: none may run
                                    // on a playback tick.
                                    crate::diag::line(serde_json::json!({"span":"timeline.paint","ms":ms,"ops":frame.ops.len()}));
                                } else {
                                    let keys = frame.ops.iter().filter(|o| o.shape.starts_with("diamond") || o.shape == "keyframe.hold").count();
                                    crate::diag::line(serde_json::json!({"span":"timeline.paint","ms":ms,"ops":frame.ops.len(),"keys":keys,"h":f32::from(bounds.size.height),
                                        "visible":frame.ops.iter().filter(|o| o.bounds.y < f64::from(f32::from(bounds.size.height)) && o.bounds.y + o.bounds.height > 0. && o.bounds.x < f64::from(f32::from(bounds.size.width)) && o.bounds.x + o.bounds.width > 0.).count()}));
                                }
                            }
                            if let Some(r) = region {
                                let (edge, alpha) = tokens.line();
                                window.paint_quad(
                                    PaintQuad::new(Bounds::new(
                                        bounds.origin + point(px(r.x as f32), px(r.y as f32)),
                                        size(px(r.width as f32), px(tokens.metrics.border)),
                                    ))
                                    .background(rgba((edge << 8) | (alpha * 255.).round() as u32)),
                                );
                            }
                            if let Some((id, x)) = &hover_clip
                                && let Some(op) = frame
                                    .ops
                                    .iter()
                                    .find(|o| o.id.strip_prefix("clip:") == Some(id.as_str()))
                            {
                                // Its rim and grip stay inside the tracks (a strip
                                // may run on out of view) and above a graph.
                                let clip = op.clip.map(|c| {
                                    Bounds::new(
                                        bounds.origin + point(px(c.x as f32), px(c.y as f32)),
                                        size(px(c.width as f32), px(c.height as f32)),
                                    )
                                });
                                let mask = match (clip, above) {
                                    (Some(c), Some(a)) => Some(c.intersect(&a)),
                                    (c, a) => c.or(a),
                                };
                                window.with_content_mask(
                                    mask.map(|bounds| ContentMask {
                                        bounds,
                                        fade_out: Default::default(),
                                    }),
                                    |w| strip_hover(op, *x, bounds.origin, &tokens, w),
                                );
                            }
                            for op in
                                pm_panel_timeline::render::decoration_ops(&frame.ops, &decorations)
                            {
                                paint(&op, bounds.origin, &tokens, window, cx);
                            }
                        }
                        for op in graph.iter().flatten() {
                            paint(op, bounds.origin, &tokens, window, cx);
                        }
                        // A graph with no curve says how to get one.
                        if let (Some(r), None) = (region, &graph) {
                            let note = graph_render::empty_note(r, &tokens);
                            paint(&note, bounds.origin, &tokens, window, cx);
                        }
                        // The bottom bar and the row thumb, over the tracks.
                        let started = crate::diag::enabled().then(std::time::Instant::now);
                        let bar = this
                            .upgrade()
                            .map(|s| s.read(cx).bar_ops())
                            .unwrap_or_default();
                        for op in &bar {
                            paint(op, bounds.origin, &tokens, window, cx);
                        }
                        if let Some(started) = started {
                            crate::diag::line(serde_json::json!({"span":"timeline.bar","ms":started.elapsed().as_secs_f64() * 1000.,"ops":bar.len()}));
                        }
                        if let Some(area) = marquee {
                            window.paint_quad(
                                PaintQuad::new(area)
                                    .border_widths(1f32)
                                    .border_color(rgb(tokens.colors.secondary)),
                            );
                        }
                        if let Some(surface) = this.upgrade() {
                            let surface = surface.read(cx);
                            if let Gesture::ParentPick { origin, at, target, moved: true, .. } = &surface.gesture {
                                // v1 installParentPickwhip: the same cubic and target label.
                                let mut line = PathBuilder::stroke(px(2.));
                                line.move_to(*origin);
                                line.cubic_bezier_to(*at, *origin + point(px(60.), px(0.)), *at - point(px(60.), px(0.)));
                                if let Ok(line) = line.build() { window.paint_path(line, rgb(tokens.colors.accent)); }
                                let label = target.as_ref().and_then(|id| surface.model.index.layer(id)).map(|l| format!("Parent to {}", l.name.to_string_lossy())).unwrap_or_else(|| "Pick a parent · Esc to cancel".into());
                                if target.is_some() {
                                    if let (Some(scene), Some(frame)) = (&surface.scene, &surface.frame) {
                                        let (_, y) = surface.local(*at);
                                        if let Some(row) = scene.layer_row_at(frame.viewport, &tokens, y) {
                                            window.paint_quad(PaintQuad::new(Bounds::new(bounds.origin + point(px(0.), px(row.top as f32)), size(bounds.size.width, px(row.height as f32)))).background(rgba((tokens.colors.accent << 8) | 0x29)));
                                        }
                                    }
                                }
                                let runs = [TextRun { len: label.len(), font: window.text_style().font(), color: rgb_to_hsla(rgb(tokens.colors.text)), background_color: None, underline: None, strikethrough: None, letter_spacing: None }];
                                let text = window.text_system().shape_line(label.into(), px(12.), &runs);
                                let label_at = point((at.x + px(12.)).min(surface.bounds.right() - px(200.)), at.y - px(12.));
                                let _ = text.paint(label_at, px(16.), TextAlign::Left, None, window, cx);
                            }
                        }
                        let moved = this.clone();
                        window.on_mouse_event(move |e: &MouseMoveEvent, phase, window, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ = moved.update(cx, |s, cx| {
                                    if s.on_move(e, window, cx) {
                                        cx.stop_propagation();
                                        return;
                                    }
                                    let over = s.over_bar(e.position, cx);
                                    if s.bounds.contains(&e.position) && !over {
                                        s.hover(e.position, cx);
                                    } else {
                                        s.set_skimmer(None, cx);
                                    }
                                });
                            }
                        });
                        let exited = this.clone();
                        window.on_mouse_event(move |_: &MouseExitEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ = exited.update(cx, |s, cx| s.set_skimmer(None, cx));
                            }
                        });
                        let released = this.clone();
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ = released.update(cx, |s, cx| {
                                    if let Gesture::ParentPick { ids, .. } = &s.gesture {
                                        let next = s.parent_target(ids, event.position);
                                        if let Gesture::ParentPick { target, at, .. } = &mut s.gesture {
                                            *target = next;
                                            *at = event.position;
                                        }
                                    }
                                    s.on_up(cx);
                                });
                            }
                        });
                    },
                )
                .size_full(),
            )
            // Over the canvas.
            .children(landing.into_iter().flatten())
            .children(reorder.into_iter().flatten().flatten())
            .children(effect_row)
            .children(tip)
            .children(self.skimmer_marks())
            .children(self.graph_readout_element())
    }
}
impl TimelineSurface {
    /// The graph's readout while keys are dragged (v1): one key reads where
    /// it is, value and time; several read the offset the whole group moved
    /// by. The Speed Graph's axis is a rate, so it reads the time alone.
    fn graph_readout_element(&self) -> Option<AnyElement> {
        if !matches!(self.gesture, Gesture::Dragging { .. }) {
            return None;
        }
        let quick = match &self.gesture {
            Gesture::Dragging {
                quick: Some(quick), ..
            } => Some(*quick),
            _ => None,
        };
        if let Some((at, total, count)) = quick {
            let local = at - self.bounds.origin;
            let t = &self.tokens;
            let left = (f32::from(local.x) + 14.)
                .max(self.view.layer_column as f32 + 8.)
                .min(
                    (f32::from(self.bounds.size.width) - 222.)
                        .max(self.view.layer_column as f32 + 8.),
                );
            let top = (f32::from(local.y) + 14.)
                .max(t.metrics.ruler + 8.)
                .min((f32::from(self.bounds.size.height) - 53.).max(t.metrics.ruler + 8.));
            let column = |left, label: &'static str, value: f64| {
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(6.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(t.colors.muted))
                            .child(label),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(t.colors.text))
                            .child(format!(
                                "{}{}",
                                if value < 0. { "−" } else { "+" },
                                crate::captions_panel::timecode(value.abs(), self.model.index.fps)
                            )),
                    )
            };
            return Some(
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(214.))
                    .h(px(45.))
                    .rounded(px(7.))
                    .bg(rgb(t.colors.floating))
                    .border(px(t.metrics.border))
                    .border_color(rgb(t.colors.separator))
                    .child(column(10., "TOTAL OFFSET", total))
                    .child(column(112., "PER LAYER", total / (count - 1) as f64))
                    .into_any_element(),
            );
        }
        let (text, x, y) = self.readout.as_ref()?;
        Some(div().absolute().left(px(*x as f32 + 14.)).top(px(*y as f32 - 26.))
            .px(px(self.tokens.metrics.gap)).py(px(2.)).rounded(px(3.))
            .bg(rgb(self.tokens.colors.accent)).text_color(rgb(0xffffff))
            .child(text.clone()).into_any_element())
    }
    /// The skimmer: a hairline of text ink across the ruler and tracks
    /// (the playhead is the accent), with its timecode on the ruler.
    /// The skimmer's x in the panel and its timecode, while it shows.
    fn skimmer_at(&self) -> Option<(f64, String)> {
        let at = self.skimmer?;
        let x = self.view.layer_column + (at - self.view.scroll_time) * self.view.pps;
        let width = f32::from(self.bounds.size.width) as f64;
        if x < self.view.layer_column || x > width {
            return None;
        }
        let fps = self.model.index.fps.max(1.);
        let frames = (at * fps).round().max(0.) as u64;
        let nominal = fps.round().max(1.) as u64;
        let sec = frames / nominal;
        let code = format!(
            "{:02}:{:02}:{:02}:{:02}",
            sec / 3600,
            sec / 60 % 60,
            sec % 60,
            frames % nominal
        );
        Some((x, code))
    }
    fn skimmer_marks(&self) -> Vec<AnyElement> {
        let Some((x, code)) = self.skimmer_at() else {
            return vec![];
        };
        let t = &self.tokens;
        let ink = (t.colors.text << 8) | 0x99;
        let line = div()
            .id("timeline-skimmer")
            .absolute()
            .left(px(x as f32 - t.metrics.border / 2.))
            .top_0()
            .bottom_0()
            .w(px(t.metrics.border))
            .bg(rgba(ink));
        let label = div()
            .absolute()
            .left(px(x as f32 + t.metrics.border * 2.))
            .top(px(t.metrics.border))
            .px(px(t.metrics.gap / 2.))
            .rounded(px(t.metrics.control_radius))
            .bg(rgb(t.colors.floating))
            .border(px(t.metrics.border))
            .border_color(rgb(t.colors.separator))
            .text_size(px(t.typography.minimum))
            .text_color(rgb(t.colors.text))
            .whitespace_nowrap()
            .child(code);
        vec![line.into_any_element(), label.into_any_element()]
    }
}
/// Where a playing head at `x` (points into a time area `width` wide)
/// should stand after a page turn, or `None` while it is in sight (v1).
fn follow_page(x: f64, width: f64) -> Option<f64> {
    if x > width - 50f64.min(width * 0.1) {
        Some(width * 0.2)
    } else if x < 0. {
        Some(width * 0.8)
    } else {
        None
    }
}
/// What the timeline's paint can leave out without changing a pixel: ops
/// wholly off the canvas, and keyframe glyphs the scene marked covered by an
/// identical one on the same device pixel (`Op::covered`). Dense keys (a
/// baked animation, a long clip zoomed out) put tens of thousands of them on
/// a few hundred columns. Hit-testing keeps every op.
struct Cull {
    width: f64,
    height: f64,
}
impl Cull {
    fn new(size: Size<Pixels>) -> Self {
        Self {
            width: f64::from(f32::from(size.width)),
            height: f64::from(f32::from(size.height)),
        }
    }
    fn skip(&self, op: &Op) -> bool {
        // Keylines and strokes reach a little past an op's box.
        const MARGIN: f64 = 4.;
        let b = op.bounds;
        op.covered
            || b.x >= self.width + MARGIN
            || b.y >= self.height + MARGIN
            || b.x + b.width <= -MARGIN
            || b.y + b.height <= -MARGIN
    }
}
/// A strip's rim (pm-design `Strip`): a ring of light inside the edge,
/// `width` wide and even all round, and light on the top edge: the strip's
/// own shape lowered by a point, shading what it leaves uncovered inside the
/// edge, so the line is full along the top, thins round the top corners and
/// is gone by the straight sides. Both are inset shadows: nothing outside the
/// edge, nothing over the body's middle, no band that starts or stops.
fn strip_rim(
    area: Bounds<Pixels>,
    radius: Pixels,
    smoothing: f32,
    ring: (u32, f32),
    width: Pixels,
    light: (u32, f32),
    window: &mut Window,
) {
    let ink = |(rgb, alpha): (u32, f32)| -> Background {
        rgba((rgb << 8) | (alpha.clamp(0., 1.) * 255.).round() as u32).into()
    };
    let corners = Corners::all(radius);
    if ring.1 > 0. {
        let shadow = BoxShadow {
            color: ink(ring),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: width,
            inset: true,
        };
        window.paint_inset_shadows(area, corners, smoothing, &[shadow]);
    }
    if light.1 > 0. {
        let shadow = BoxShadow {
            color: ink(light),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.5),
            spread_radius: px(0.),
            inset: true,
        };
        window.paint_inset_shadows(area, corners, smoothing, &[shadow]);
    }
}
/// The strip under the pointer: its rim in the hover state (lighter on the
/// dark canvas, deeper on the light one; nothing else changes) and the trim grip stands inside the nearer edge.
fn strip_hover(op: &Op, pointer_x: f64, origin: Point<Pixels>, t: &Tokens, window: &mut Window) {
    let b = op.bounds;
    let area = Bounds::new(
        origin + point(px(b.x as f32), px(b.y as f32)),
        size(px(b.width as f32), px(b.height as f32)),
    );
    let hover = t.strip(op.color, pm_design::StripState::Hover);
    let radius = px(op.radius as f32).min(area.size.height / 2.);
    strip_rim(
        area,
        radius,
        t.radius.smoothing,
        hover.ring,
        px(hover.ring_width),
        hover.light,
        window,
    );
    let grip = t.trim_grip();
    // A locked layer cannot be trimmed: no grip offers it.
    let locked = op.detail["cursor"] == "notAllowed";
    if locked || b.width < f64::from(grip.inset * 4. + grip.width) {
        return;
    }
    let height = (b.height as f32 * grip.height).round();
    let x = if pointer_x < b.x + b.width / 2. {
        area.origin.x + px(grip.inset)
    } else {
        area.right() - px(grip.inset + grip.width)
    };
    window.paint_quad(
        PaintQuad::new(Bounds::new(
            point(x, area.origin.y + (area.size.height - px(height)) / 2.),
            size(px(grip.width), px(height)),
        ))
        .background(rgb(grip.color))
        .corner_radii(px(grip.width / 2.)),
    );
}
fn paint(op: &Op, origin: Point<Pixels>, t: &Tokens, window: &mut Window, cx: &mut App) {
    let at = |x: f64, y: f64| origin + point(px(x as f32), px(y as f32));
    let b = op.bounds;
    let area = Bounds::new(at(b.x, b.y), size(px(b.width as f32), px(b.height as f32)));
    let alpha = (op.opacity.clamp(0., 1.) * 255.).round() as u32;
    let color = rgba((op.color << 8) | alpha);
    let clip = op
        .clip
        .map(|c| Bounds::new(at(c.x, c.y), size(px(c.width as f32), px(c.height as f32))));
    let draw = |window: &mut Window, cx: &mut App| match op.shape {
        "stagger" => {
            let mut path = PathBuilder::fill();
            for (i, p) in op.detail["points"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let point = at(p[0].as_f64().unwrap_or(0.), p[1].as_f64().unwrap_or(0.));
                if i == 0 {
                    path.move_to(point);
                } else {
                    path.line_to(point);
                }
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
        // 45° stripes clipped to the op's rounded box (offline clips, the
        // render bar while rendering).
        "stripes-45" => {
            let period = op.detail["period"].as_f64().unwrap_or(6.).max(2.) as f32;
            let stripe = op.detail["stripe"].as_f64().unwrap_or(2.).max(0.5) as f32;
            let (x0, y0) = (f32::from(area.origin.x), f32::from(area.origin.y));
            let (w, h) = (f32::from(area.size.width), f32::from(area.size.height));
            window.with_content_mask(
                Some(ContentMask {
                    bounds: area,
                    fade_out: Default::default(),
                }),
                |window| {
                    let mut x = x0 - h;
                    while x < x0 + w {
                        let mut path = PathBuilder::stroke(px(stripe));
                        path.move_to(point(px(x), px(y0 + h)));
                        path.line_to(point(px(x + h), px(y0)));
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                        x += period;
                    }
                },
            );
        }
        // A layer or clip strip (pm-design `strip`): a slab of tinted glass.
        // A deep top opening into a luminous lower half, the ruler's grid
        // reading through the body, and a rim of light inside the edge that
        // is brightest along the top and round its corners.
        "clip" => {
            let hex = |key: &str| op.detail[key].as_u64().unwrap_or(op.color as u64) as u32;
            let number = |key: &str| op.detail[key].as_f64().unwrap_or(0.);
            let tint =
                |rgb: u32, a: f64| rgba((rgb << 8) | (a.clamp(0., 1.) * 255.).round() as u32);
            let radius = px(op.radius as f32).min(area.size.height / 2.);
            let smoothing = t.radius.smoothing;
            let ring = px(number("ringWidth").max(1.) as f32);
            if number("shadowAlpha") > 0. {
                window.paint_drop_shadows(
                    area,
                    Corners::all(radius),
                    smoothing,
                    &[BoxShadow {
                        color: tint(hex("shadow"), number("shadowAlpha")).into(),
                        offset: point(px(0.), px(number("shadowY") as f32)),
                        blur_radius: px(number("shadowBlur") as f32),
                        spread_radius: px(0.),
                        inset: false,
                    }],
                );
            }
            // The body: one ramp from the deep top to the luminous foot.
            window.paint_quad(
                PaintQuad::new(area)
                    .background(linear_gradient(
                        180.,
                        linear_color_stop(tint(hex("top"), op.opacity), 0.),
                        linear_color_stop(tint(hex("bottom"), op.opacity), 1.),
                    ))
                    .corner_radii(radius)
                    .corner_smoothing(smoothing),
            );
            let (step, grid) = (number("gridStep") as f32, number("gridAlpha"));
            if step >= 8. && grid > 0. {
                let mut x = origin.x + px(number("gridFrom") as f32);
                let (left, right) = (area.origin.x + radius, area.right() - radius);
                while x < right {
                    if x > left {
                        // A highlight lit from the strip's top edge, gone
                        // by the middle of its height.
                        window.paint_quad(
                            PaintQuad::new(Bounds::new(
                                point(x, area.origin.y + ring),
                                size(
                                    px(1.),
                                    (area.size.height - ring * 2.) * pm_design::GRID_FADE,
                                ),
                            ))
                            .background(linear_gradient(
                                180.,
                                linear_color_stop(tint(hex("grid"), grid), 0.),
                                linear_color_stop(tint(hex("grid"), 0.), 1.),
                            )),
                        );
                    }
                    x += px(step);
                }
            }
            strip_rim(
                area,
                radius,
                smoothing,
                (hex("ring"), number("ringAlpha") as f32),
                ring,
                (hex("light"), number("lightAlpha") as f32),
                window,
            );
        }
        "square" => {
            if let Some(keyline) = op.detail["keyline"].as_u64() {
                let w = px(op.detail["keylineWidth"].as_f64().unwrap_or(1.) as f32);
                window.paint_quad(
                    PaintQuad::new(area.dilate(w))
                        .background(rgb(keyline as u32))
                        .corner_radii(px(1.)),
                );
            }
            window.paint_quad(PaintQuad::new(area).background(color).corner_radii(px(1.)));
        }
        "rect" | "rounded-rect" | "work-area-handle" => {
            window.paint_quad(
                PaintQuad::new(area)
                    .background(color)
                    .corner_radii(px(op.radius as f32)),
            );
        }
        // v1's transform box: a dashed edge.
        "ring" if op.detail["dash"].is_array() => {
            let (on, off) = (
                op.detail["dash"][0].as_f64().unwrap_or(3.).max(0.5),
                op.detail["dash"][1].as_f64().unwrap_or(3.).max(0.),
            );
            let (x0, y0) = (b.x.round() + 0.5, b.y.round() + 0.5);
            let (x1, y1) = (x0 + b.width.round(), y0 + b.height.round());
            let mut path = PathBuilder::stroke(px(op.stroke.max(1.) as f32));
            for (from, to) in [
                ((x0, y0), (x1, y0)),
                ((x1, y0), (x1, y1)),
                ((x1, y1), (x0, y1)),
                ((x0, y1), (x0, y0)),
            ] {
                let (dx, dy) = (to.0 - from.0, to.1 - from.1);
                let length = dx.hypot(dy);
                let mut d = 0.;
                while d < length {
                    let e = (d + on).min(length);
                    let p = |f: f64| at(from.0 + dx * f / length, from.1 + dy * f / length);
                    path.move_to(p(d));
                    path.line_to(p(e));
                    d = e + off;
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
        "ring" | "outside-ring" => {
            let grown = if op.shape == "ring" {
                area
            } else {
                area.dilate(px(op.stroke as f32))
            };
            window.paint_quad(
                PaintQuad::new(grown)
                    .border_color(color)
                    .border_widths(px(op.stroke as f32))
                    .corner_radii(px(op.radius as f32)),
            );
        }
        "circle" => {
            if let Some(keyline) = op.detail["keyline"].as_u64() {
                let grown =
                    area.dilate(px(op.detail["keylineWidth"].as_f64().unwrap_or(1.) as f32));
                window.paint_quad(
                    PaintQuad::new(grown)
                        .background(rgb(keyline as u32))
                        .corner_radii(grown.size.width / 2.),
                );
            }
            window.paint_quad(
                PaintQuad::new(area)
                    .background(color)
                    .corner_radii(area.size.width / 2.),
            );
        }
        "diamond" | "diamond.fill" | "diamond.lefthalf.filled" | "keyframe.hold" => {
            if let Some(keyline) = op.detail["keyline"].as_u64() {
                let w = px(op.detail["keylineWidth"].as_f64().unwrap_or(1.) as f32 * 1.5);
                diamond(area.dilate(w), rgb(keyline as u32), window);
            }
            match op.detail["half"].as_str() {
                // The end of a span: an outline with the half toward the span filled.
                Some(side) => {
                    diamond_outline(area, color, window);
                    half_diamond(area, side == "right", color, window);
                }
                None => diamond(area, color, window),
            }
        }
        // A marker pin: flat top, pointed foot (v1's marker).
        "marker" => {
            let (l, r) = (area.origin.x, area.origin.x + area.size.width);
            let (top, bottom) = (area.origin.y, area.origin.y + area.size.height);
            let mid = area.center().x;
            let shoulder = bottom - area.size.width / 2.;
            let mut path = PathBuilder::fill();
            path.move_to(point(l, top));
            path.line_to(point(r, top));
            path.line_to(point(r, shoulder));
            path.line_to(point(mid, bottom));
            path.line_to(point(l, shoulder));
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
        // The playhead pin (v1's): a flat top, straight sides and a point
        // resting on the foot of the ruler.
        "ph-head" => {
            if let Ok(path) = playhead_pin(area).build() {
                window.paint_path(path, color);
            }
        }
        "line" => {
            if let (Some(from), Some(to)) =
                (point_of(&op.detail["from"]), point_of(&op.detail["to"]))
            {
                let mut path = PathBuilder::stroke(px(op.stroke as f32));
                // `dash: [on, off]`: v1's dashed tangents.
                let dash = op.detail["dash"]
                    .as_array()
                    .and_then(|d| Some((d.first()?.as_f64()?, d.get(1)?.as_f64()?)))
                    .filter(|(on, off)| *on > 0. && *off >= 0.);
                match dash {
                    Some((on, off)) => {
                        let (dx, dy) = (to.0 - from.0, to.1 - from.1);
                        let length = dx.hypot(dy);
                        let mut d = 0.;
                        while d < length {
                            let e = (d + on).min(length);
                            let p = |f: f64| at(from.0 + dx * f / length, from.1 + dy * f / length);
                            path.move_to(p(d));
                            path.line_to(p(e));
                            d = e + off;
                        }
                    }
                    None => {
                        path.move_to(at(from.0, from.1));
                        path.line_to(at(to.0, to.1));
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        }
        // A key in the graph (v1's `drawGraphVertex`): its interpolation's
        // shape in the curve's colour, ringed in green when selected.
        "graph-vertex" => {
            let centre = area.center();
            let r = f32::from(area.size.width) / 2.;
            let kind = op.detail["kind"].as_str().unwrap_or("linear");
            let outline = |radius: f32| {
                let p = |x: f32, y: f32| centre + point(px(x), px(y));
                match kind {
                    "hold" => vec![
                        p(-radius, -radius),
                        p(radius, -radius),
                        p(radius, radius),
                        p(-radius, radius),
                    ],
                    "bezier" => vec![
                        p(-radius, -radius),
                        p(radius, -radius),
                        p(-radius, radius),
                        p(radius, radius),
                    ],
                    _ => vec![
                        p(0., -radius - 1.),
                        p(radius + 1., 0.),
                        p(0., radius + 1.),
                        p(-radius - 1., 0.),
                    ],
                }
            };
            if let Some(ring) = op.detail["ring"].as_u64() {
                let corners = outline(r + 2.);
                let mut path = PathBuilder::stroke(px(1.3));
                path.move_to(corners[0]);
                for c in &corners[1..] {
                    path.line_to(*c);
                }
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(ring as u32));
                }
            }
            let corners = outline(r);
            let mut path = PathBuilder::fill();
            path.move_to(corners[0]);
            for c in &corners[1..] {
                path.line_to(*c);
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
        "curve" => {
            let mut path = PathBuilder::stroke(px(op.stroke as f32));
            let mut started = false;
            for segment in op.detail["segments"].as_array().into_iter().flatten() {
                if let (Some(from), Some(to)) =
                    (point_of(&segment["from"]), point_of(&segment["to"]))
                {
                    if !started {
                        path.move_to(at(from.0, from.1));
                        started = true;
                    }
                    // Eased segments are the Bézier they evaluate, not a chord.
                    match segment["controls"]
                        .as_array()
                        .and_then(|c| Some((point_of(c.first()?)?, point_of(c.get(1)?)?)))
                    {
                        Some((a, b)) => {
                            path.cubic_bezier_to(at(to.0, to.1), at(a.0, a.1), at(b.0, b.1))
                        }
                        None => path.line_to(at(to.0, to.1)),
                    }
                }
            }
            if started && let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
        // Bars stand on the clip's floor, one point per column; while the
        // peaks are read a hairline marks the middle (v1).
        "waveform" => {
            let bars = op.detail["bars"].as_array();
            let inset = op.detail["inset"].as_f64().unwrap_or(2.) as f32;
            let min_bar = op.detail["minBar"].as_f64().unwrap_or(1.) as f32;
            let (x0, top) = (f32::from(area.origin.x), f32::from(area.origin.y));
            let (w, h) = (f32::from(area.size.width), f32::from(area.size.height));
            let room = (h - 2. * inset).max(0.);
            match bars.filter(|b| !b.is_empty()) {
                None => window.paint_quad(
                    PaintQuad::new(Bounds::new(
                        point(px(x0), px(top + (h / 2.).floor())),
                        size(px(w), px(1.)),
                    ))
                    .background(color),
                ),
                Some(bars) => {
                    let floor = top + h - inset;
                    let mut path = PathBuilder::fill();
                    path.move_to(point(px(x0), px(floor)));
                    // Silence draws nothing: a bar shorter than `minBar`
                    // would run along the foot as an underline.
                    for (i, peak) in bars.iter().enumerate() {
                        let bar = peak.as_f64().unwrap_or(0.) as f32 * room;
                        let bar = if bar < min_bar { 0. } else { bar };
                        let x = x0 + i as f32;
                        path.line_to(point(px(x), px(floor - bar)));
                        path.line_to(point(px(x + 1.), px(floor - bar)));
                    }
                    path.line_to(point(px(x0 + bars.len() as f32), px(floor)));
                    path.close();
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                }
            }
        }
        "text" => {
            let Some(text) = &op.text else {
                return;
            };
            if text.is_empty() {
                return;
            }
            let run = TextRun {
                len: text.len(),
                font: Font {
                    family: pm_design::TEXT_FONT.into(),
                    features: FontFeatures(Arc::new(vec![("tnum".into(), 1)])),
                    ..window.text_style().font()
                },
                color: rgb_to_hsla(color),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            };
            let font_size = px(op.font_size as f32);
            let shape = |text: String, window: &mut Window| {
                let run = TextRun {
                    len: text.len(),
                    ..run.clone()
                };
                window
                    .text_system()
                    .shape_line(text.into(), font_size, &[run])
            };
            let mut line = shape(text.clone(), window);
            // A clip's name ends in an ellipsis inside the clip, as in every NLE,
            // keeping the same padding on the right as on the left.
            if op.detail["truncate"] == "tail" {
                let room = px(
                    (b.width - (t.metrics.inset - t.metrics.inner_radius) as f64).max(0.) as f32,
                );
                if line.width > room {
                    let Some(fitted) = fit_tail(text, |s| shape(s.into(), window).width <= room)
                    else {
                        return;
                    };
                    // A strip too short for three letters of its name shows
                    // its colour alone: one letter and an ellipsis names
                    // nothing.
                    if fitted.chars().count() < NAME_LEAST + 1 {
                        return;
                    }
                    line = shape(fitted, window);
                }
            }
            let height = font_size * t.typography.dense_leading;
            let y = b.y + (b.height - f32::from(height) as f64) / 2.;
            // Centred labels (the ruler's) keep clear of `minX`.
            let x = if op.detail["align"] == "center" {
                let x = b.x + (b.width - f32::from(line.width) as f64) / 2.;
                x.max(op.detail["minX"].as_f64().unwrap_or(f64::MIN))
            } else if op.detail["align"] == "right" {
                b.x + b.width - f32::from(line.width) as f64
            } else {
                b.x
            };
            let _ = line.paint(at(x, y.max(b.y)), height, TextAlign::Left, None, window, cx);
        }
        _ => {}
    };
    match clip {
        Some(bounds) => window.with_content_mask(
            Some(ContentMask {
                bounds,
                fade_out: Default::default(),
            }),
            |w| draw(w, cx),
        ),
        None => draw(window, cx),
    }
}
/// The longest start of `text` that, with a trailing ellipsis, `fits`; none when
/// not even one character does.
pub(crate) fn fit_tail(text: &str, mut fits: impl FnMut(&str) -> bool) -> Option<String> {
    let ends: Vec<usize> = text.char_indices().map(|(i, c)| i + c.len_utf8()).collect();
    let candidate = |n: usize| format!("{}…", text[..ends[n - 1]].trim_end());
    let (mut keep, mut over) = (0, ends.len());
    while keep + 1 < over {
        let mid = (keep + over) / 2;
        if fits(&candidate(mid)) {
            keep = mid;
        } else {
            over = mid;
        }
    }
    (keep > 0).then(|| candidate(keep))
}
fn point_of(value: &serde_json::Value) -> Option<(f64, f64)> {
    Some((value.get(0)?.as_f64()?, value.get(1)?.as_f64()?))
}
fn diamond(area: Bounds<Pixels>, color: Rgba, window: &mut Window) {
    let c = area.center();
    let (w, h) = (area.size.width / 2., area.size.height / 2.);
    convex(
        &[
            point(c.x, c.y - h),
            point(c.x + w, c.y),
            point(c.x, c.y + h),
            point(c.x - w, c.y),
        ],
        color,
        window,
    );
}
/// A convex polygon filled as a fan of triangles: the same coverage a
/// tessellator gives it, without one per keyframe glyph (a dense timeline
/// paints tens of thousands of them a frame).
fn convex(corners: &[Point<Pixels>], color: Rgba, window: &mut Window) {
    let mut path = gpui::Path::new(corners[0]);
    for pair in corners[1..].windows(2) {
        path.push_triangle(
            (corners[0], pair[0], pair[1]),
            (point(0., 1.), point(0., 1.), point(0., 1.)),
        );
    }
    window.paint_path(path, color);
}
fn diamond_outline(area: Bounds<Pixels>, color: Rgba, window: &mut Window) {
    let c = area.center();
    let (w, h) = (area.size.width / 2., area.size.height / 2.);
    let mut path = PathBuilder::stroke(px(1.));
    path.move_to(point(c.x, c.y - h));
    path.line_to(point(c.x + w, c.y));
    path.line_to(point(c.x, c.y + h));
    path.line_to(point(c.x - w, c.y));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
/// The right (`toward_right`) or left half of a diamond, filled.
fn half_diamond(area: Bounds<Pixels>, toward_right: bool, color: Rgba, window: &mut Window) {
    let c = area.center();
    let (w, h) = (area.size.width / 2., area.size.height / 2.);
    let tip = if toward_right { c.x + w } else { c.x - w };
    convex(
        &[point(c.x, c.y - h), point(tip, c.y), point(c.x, c.y + h)],
        color,
        window,
    );
}

#[cfg(test)]
mod cull_tests {
    use super::Cull;
    use gpui::{px, size};
    use pm_panel_timeline::render::{Op, Rect};

    fn op(id: &str, x: f64, y: f64, covered: bool) -> Op {
        Op {
            id: id.into(),
            shape: "diamond",
            bounds: Rect::new(x, y, 9., 9.),
            color: 0xffffff,
            opacity: 0.85,
            radius: 0.,
            stroke: 0.,
            text: None,
            font_size: 0.,
            detail: serde_json::json!({"half": null}),
            clip: None,
            covered,
        }
    }

    #[test]
    fn a_playing_head_turns_the_page_at_the_edges_as_v1() {
        // In sight: no page turn.
        assert_eq!(super::follow_page(0., 1000.), None);
        assert_eq!(super::follow_page(949., 1000.), None);
        // Within 50 points of the right edge: back to a fifth of the way in.
        assert_eq!(super::follow_page(951., 1000.), Some(200.));
        // A narrow area turns a tenth from its edge.
        assert_eq!(super::follow_page(181., 200.), Some(40.));
        assert_eq!(super::follow_page(179., 200.), None);
        // Behind the left edge (a loop, or backwards): four fifths in.
        assert_eq!(super::follow_page(-1., 1000.), Some(800.));
    }
    #[test]
    fn only_covered_and_off_canvas_ops_are_left_out() {
        let cull = Cull::new(size(px(400.), px(100.)));
        assert!(!cull.skip(&op("key:[1,a]", 50., 10., false)));
        assert!(cull.skip(&op("key:[1,b]", 50.2, 10., true)));
        // Wholly off the canvas: nothing to draw, key or not.
        assert!(cull.skip(&op("key:[9,a]", 50., 140., false)));
        assert!(cull.skip(&op("clip:y", 500., 10., false)));
        // Partly on the canvas draws.
        assert!(!cull.skip(&op("clip:z", 395., 10., false)));
    }
}

/// The playhead pin in `area` (v1's 11 pt pentagon: the shoulders sit at
/// six elevenths of its height, then the sides meet in a point).
fn playhead_pin(area: Bounds<Pixels>) -> PathBuilder {
    let (l, r) = (area.origin.x, area.origin.x + area.size.width);
    let (top, bottom) = (area.origin.y, area.origin.y + area.size.height);
    let shoulder = top + area.size.height * (6. / 11.);
    let mut path = PathBuilder::fill();
    path.move_to(point(l, top));
    path.line_to(point(r, top));
    path.line_to(point(r, shoulder));
    path.line_to(point(area.center().x, bottom));
    path.line_to(point(l, shoulder));
    path.close();
    path
}

/// The layer column a timeline opens with: the theme's, at least the least.
pub fn default_layer_column(m: &pm_design::Metrics) -> f64 {
    (m.layer_column as f64).max(least_layer_column(m))
}
/// v1's least layer column (`GUTTER_MIN`, 320 pt).
const V1_LEAST_LAYER_COLUMN: f64 = 320.;
/// The narrowest the layer column goes: a layer row's number, disclosure,
/// kind badge, three slots of name and its three switches (parent link,
/// lock, visibility), with its insets and gaps, and never narrower than
/// v1's. The name truncates first; the switches always fit.
pub fn least_layer_column(m: &pm_design::Metrics) -> f64 {
    let (gap, hit) = (m.gap as f64, m.hit as f64);
    // Insets and the gaps between number, disclosure, badge and name.
    let room = 2. * gap + 4. * gap;
    let row = room + hit + hit + m.kind_badge as f64 + 3. * hit + 3. * hit;
    row.max(V1_LEAST_LAYER_COLUMN).ceil()
}

/// How far either side of a work-area edge its handle takes hold (pt): a
/// 12 pt zone over the ruler's height, as v1's (WORK_BAR.hit).
const WORK_HANDLE_REACH: f64 = 6.;
/// v1's work-area bar: the top strip of the ruler a plain drag moves it by
/// (WORK_BAR.height with a point of grace).
const WORK_BAR_STRIP: f64 = 6.;

// Scrolling follows the pointer crossing the visible time area, never a
// composition/work-area endpoint or the dragged object's bounds.
fn edge_overflow(pointer: f32, left: f32, right: f32) -> f32 {
    if pointer < left {
        pointer - left
    } else if pointer > right {
        pointer - right
    } else {
        0.
    }
}
#[cfg(test)]
mod edge_scroll_regression {
    use super::edge_overflow;
    #[test]
    fn only_the_visible_edge_starts_scrolling() {
        for pointer in [200., 400., 700., 1000.] {
            assert_eq!(edge_overflow(pointer, 200., 1000.), 0.);
        }
        assert_eq!(edge_overflow(199., 200., 1000.), -1.);
        assert_eq!(edge_overflow(1001., 200., 1000.), 1.);
    }
}

pub(crate) const EMPTY_CAMERA_COMMAND: &str = "newCamera";
pub(crate) fn empty_new_menu() -> serde_json::Value {
    serde_json::json!({"label":"New","children":[{"label":"Camera","command":EMPTY_CAMERA_COMMAND}]})
}
