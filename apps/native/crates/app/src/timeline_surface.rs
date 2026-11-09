//! Native timeline host element. Pointer input runs Rust gesture sessions against
//! the kernel on a worker; JS keeps the outline and hears about it by publication.
//! No sample waits on V8, and the main thread never waits on the kernel.
use crate::v4_view::Outline;
use gpui::{prelude::*, *};
use pm_design::Tokens;
use pm_kernel::{Caller, KernelHandle, Published, QueryMirror, SelectionCall};
use pm_panel_timeline::{
    Context as MenuContext, DragKind, DragSample, GraphDisplay, HostEffect, MarqueeSession,
    Modifiers as Mods, Plan, PropertyFacts, RowId, Scrub, TimelineClient, TimelineMode,
    TimelineModel, frame,
    graph_render::{self, Graph},
    render::{Edge, Frame, Interpolation, Label, Media, ModelPresenter, Op, Rect, Scene, Viewport},
};
use std::{
    cell::RefCell,
    collections::{BTreeSet, VecDeque},
    rc::Rc,
    sync::Arc,
};

/// Host services a gesture needs. Implemented by the Controller; tests record.
pub trait Delegate: 'static {
    fn time(&self, cx: &App) -> f64;
    fn seek(&self, seconds: f64, cx: &mut App);
    fn effect(&self, effect: HostEffect, cx: &mut App);
    fn report(&self, message: String, cx: &mut App);
    /// Run a shell command by name, as its menu item or chord would.
    fn command(&self, name: &str, cx: &mut App);
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
    Trim(Edge),
    Offset,
    Ripple(Edge),
    Roll(Edge),
    Slip,
    Slide,
    Keys,
    GraphKeys,
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
}
struct Press {
    origin: Point<Pixels>,
    spec: Option<Spec>,
    /// A plain click on an already selected item narrows selection on release.
    release: Option<Job>,
}
enum Gesture {
    Idle,
    Pressed(Press),
    Dragging {
        origin: Point<Pixels>,
        proedit: bool,
        readout: bool,
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
    /// Assets this editor's viewer cannot read: their clips show Media
    /// Offline (docs/native/design/media-offline.md).
    offline: Arc<BTreeSet<String>>,
    model: Arc<TimelineModel>,
    scene: Option<Arc<Scene>>,
    graph: Option<Arc<Graph>>,
    frame: Option<Arc<Frame>>,
    bounds: Bounds<Pixels>,
    view: Viewed,
    /// The native playhead's time mapping: pixels per second, first second and
    /// time column of the frame last prepared.
    shown: (f64, f64, f64),
    fitted: bool,
    gesture: Gesture,
    cursor: CursorStyle,
    /// A badge's tooltip shown after `motion.tooltip_delay` (text, local point).
    tip: Option<(SharedString, (f64, f64))>,
    tip_wait: Option<Task<()>>,
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
}
const WHEEL_ZOOM: f64 = 0.01;
const PPS: (f64, f64) = (4., 4000.);
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
        let layer_column = tokens.metrics.layer_column as f64;
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
            offline: Arc::new(offline),
            model: Default::default(),
            scene: None,
            graph: None,
            frame: None,
            bounds: Bounds::default(),
            view,
            shown: (view.pps, view.scroll_time, view.layer_column),
            fitted: false,
            gesture: Gesture::Idle,
            cursor: CursorStyle::Arrow,
            tip: None,
            tip_wait: None,
            idle_persist: None,
            decorations: Default::default(),
            painted: None,
            painting: false,
            told: None,
            late: None,
        };
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
        cx.spawn(async move |this, cx| {
            // Indexing the document is O(layers): never on the main thread.
            let created = cx
                .background_executor()
                .spawn(async move {
                    TimelineClient::new(kernel, project, Caller::native()).map(|mut client| {
                        let _ = client.adopt_view();
                        let built = build(&client, &tokens, offline);
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
    pub fn set_tokens(&mut self, tokens: Tokens, cx: &mut Context<Self>) {
        if self.tokens != tokens {
            // A layer column the person never dragged follows the theme's
            // (workspace timeline chrome); a dragged one stays.
            let (old, new) = (
                self.tokens.metrics.layer_column as f64,
                tokens.metrics.layer_column as f64,
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
        self.queue.push_back(Job::Command {
            id: id.into(),
            time,
            frames,
            shift,
        });
        self.pump(cx);
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
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Idle => return false,
            Gesture::Dragging { .. } => self.queue.push_back(Job::Finish(false)),
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
            self.view.pps = (width / self.model.index.duration).clamp(PPS.0, PPS.1);
            self.view.scroll_time = 0.;
            self.changed(true, cx);
        }
    }
    fn zoom_at(&mut self, x: f64, factor: f64, cx: &mut Context<Self>) {
        let anchor = self.view.scroll_time + (x - self.view.layer_column) / self.view.pps;
        self.view.pps = (self.view.pps * factor).clamp(PPS.0, PPS.1);
        self.view.scroll_time = (anchor - (x - self.view.layer_column) / self.view.pps)
            .max(self.model.index.minimum_time());
        self.fitted = true;
        self.changed_while_hot(cx);
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
        let (tokens, offline) = (self.tokens.clone(), self.offline.clone());
        cx.spawn(async move |this, cx| {
            let (client, effects, built) = cx
                .background_executor()
                .spawn(async move {
                    let effects = run(&mut client, job).await;
                    let built = build(&client, &tokens, offline);
                    (client, effects, built)
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.client = Some(client);
                s.busy = false;
                s.install(built, cx);
                match effects {
                    Ok(effects) => {
                        for effect in effects {
                            match effect {
                                HostEffect::Seek(time) => s.delegate.seek(time, cx),
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
        self.model = built.model;
        self.scene = Some(built.scene.clone());
        // A drag maps the pointer through the graph's value range: hold it until
        // release so the curve does not slide out from under the pointer.
        self.graph = match (&self.gesture, &self.graph, built.graph) {
            (Gesture::Dragging { .. }, Some(held), Some(next)) => {
                let mut next = Arc::unwrap_or_clone(next);
                (next.min, next.max) = (held.min, held.max);
                Some(Arc::new(next))
            }
            (_, _, next) => next,
        };
        self.outline.set_scene(Some(built.scene));
        self.frame = None;
        self.clamp_scroll();
        if !self.fitted && self.model.index.duration > 0. && self.bounds.size.width > px(0.) {
            self.fitted = true;
            let width = (f32::from(self.bounds.size.width) as f64 - self.view.layer_column).max(1.);
            self.view.pps = (width / self.model.index.duration).clamp(PPS.0, PPS.1);
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
        if self.bounds != bounds {
            self.bounds = bounds;
            self.frame = None;
            if !self.fitted && self.model.index.duration > 0. {
                self.fit_time(cx);
            }
            self.clamp_scroll();
        }
        if self.frame.is_none() {
            self.shown = (self.view.pps, self.view.scroll_time, self.view.layer_column);
            let (scene, viewport) = (self.scene.as_ref()?, self.viewport()?);
            self.frame = scene.prepare(viewport, &self.tokens).ok().map(Arc::new);
            if let Some(frame) = self.frame.clone() {
                self.ask_painters(&frame, cx);
            }
            self.tell_service(cx);
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
                    Ok(RowId::Property { layer_id, .. }) => layer_id,
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
    fn time_at(&self, x: f64) -> f64 {
        self.view.scroll_time + (x - self.view.layer_column) / self.view.pps
    }
    fn graph_area(&self) -> Option<Rect> {
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
            GraphDisplay::Combined => Some(Rect::new(x, ruler + body / 2., w - x, body / 2.)),
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
        let Some(frame) = self.frame.clone() else {
            return;
        };
        let m = self.tokens.metrics;
        let column = self.view.layer_column;
        if (x - column).abs() <= m.hit as f64 / 4. && y >= m.ruler as f64 {
            if event.click_count == 2 {
                self.view.layer_column = m.layer_column as f64;
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
        let additive = mods.shift || mods.command();
        if y < m.ruler as f64 {
            for (prefix, spec) in [("work-in", Spec::WorkIn), ("work-out", Spec::WorkOut)] {
                if frame
                    .hit_op(prefix, x, y, m.drag_threshold as f64)
                    .is_some()
                {
                    self.gesture = Gesture::Pressed(Press {
                        origin: event.position,
                        spec: Some(spec),
                        release: None,
                    });
                    return;
                }
            }
            if frame.hit_op("work-area", x, y, 0.).is_some() && mods.alt {
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
        if let Some(area) = self.graph_area()
            && y >= area.y
        {
            self.press_graph(event, x, y, additive, area, cx);
            return;
        }
        let slop = ((m.hit - m.keyframe) as f64 / 2.).max(0.);
        if let Some((_, key)) = frame.hit_key(x, y, slop)
            && let Some((layer, channel, _)) = self.model.index.key(&key)
        {
            let selected = self.model.selection.keys().contains(&key);
            let (layer, channel) = (layer.id.clone(), channel.path.clone());
            self.queue.push_back(Job::SelectKey {
                layer: layer.clone(),
                channel: channel.clone(),
                key: key.clone(),
                additive,
                dragging: true,
            });
            self.gesture = Gesture::Pressed(Press {
                origin: event.position,
                spec: Some(Spec::Keys),
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
            let spec = match edge {
                Some(edge) => {
                    let edge = match edge {
                        pm_panel_timeline::render::Edge::In => Edge::In,
                        pm_panel_timeline::render::Edge::Out => Edge::Out,
                    };
                    if tracks && mods.alt {
                        Spec::Ripple(edge)
                    } else if tracks && mods.control {
                        Spec::Roll(edge)
                    } else {
                        Spec::Trim(edge)
                    }
                }
                None if mods.command() && mods.alt => Spec::Offset,
                None if tracks && mods.control => Spec::Slip,
                None if tracks && mods.alt => Spec::Slide,
                None => Spec::Layers,
            };
            let selected = self.model.selection.contains(&layer);
            // Pro-edit modifiers choose the tool; they must not toggle selection.
            let picking = if matches!(
                spec,
                Spec::Ripple(_) | Spec::Roll(_) | Spec::Slip | Spec::Slide
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
    pub fn press_property(&mut self, layer: String, path: String, origin: Point<Pixels>) {
        self.gesture = Gesture::Pressed(Press {
            origin,
            spec: Some(Spec::Property { layer, path }),
            release: None,
        });
    }
    fn press_graph(
        &mut self,
        event: &MouseDownEvent,
        x: f64,
        y: f64,
        additive: bool,
        area: Rect,
        cx: &mut Context<Self>,
    ) {
        let ops = self.graph_ops(area).unwrap_or_default();
        let hit = |prefix: &str| {
            ops.iter()
                .rev()
                .find(|o| {
                    o.id.starts_with(prefix)
                        && x >= o.bounds.x - 3.
                        && x <= o.bounds.x + o.bounds.width + 3.
                        && y >= o.bounds.y - 3.
                        && y <= o.bounds.y + o.bounds.height + 3.
                })
                .map(|o| o.id[prefix.len()..].to_owned())
        };
        // Handles sit over keys and other curves: they win.
        if let Some((key, side)) = hit("graph-handle:").and_then(|h| {
            let (key, side) = h.rsplit_once(':')?;
            let side = match side {
                "in" => pm_kernel::commands::keyframes::EaseSide::In,
                _ => pm_kernel::commands::keyframes::EaseSide::Out,
            };
            Some((key.to_owned(), side))
        }) {
            self.gesture = Gesture::Pressed(Press {
                origin: event.position,
                spec: Some(Spec::Handle { key, side }),
                release: None,
            });
            return;
        }
        if let Some(key) = hit("graph-key:")
            && let Some((layer, channel, _)) = self.model.index.key(&key)
        {
            self.queue.push_back(Job::SelectKey {
                layer: layer.id.clone(),
                channel: channel.path.clone(),
                key,
                additive,
                dragging: true,
            });
            self.gesture = Gesture::Pressed(Press {
                origin: event.position,
                spec: Some(Spec::GraphKeys),
                release: None,
            });
            self.pump(cx);
        } else {
            self.gesture = Gesture::Marquee {
                session: MarqueeSession::begin(&self.model, additive, true),
                origin: event.position,
                at: event.position,
            };
        }
    }
    fn graph_ops(&self, area: Rect) -> Option<Vec<Op>> {
        self.graph
            .as_ref()?
            .prepare(self.viewport()?, area, &self.tokens)
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
    fn on_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        match &mut self.gesture {
            Gesture::Idle => return false,
            Gesture::Pressed(press) => {
                let delta = event.position - press.origin;
                let travel = f32::from(delta.x).hypot(f32::from(delta.y));
                if travel < self.tokens.metrics.drag_threshold {
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
                let handle = match &spec {
                    Spec::Handle { key, .. } => self.model.index.key(key).map(|(l, _, _)| l.from),
                    _ => None,
                };
                let time = self.delegate.time(cx);
                self.queue.push_back(Job::Begin(spec, time));
                self.gesture = Gesture::Dragging {
                    origin,
                    proedit,
                    readout,
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
                handle,
            } => {
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
                self.autoscroll(event.position, cx);
                self.queue.push_back(Job::Sample(sample));
                self.pump(cx);
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
                self.view.scroll_time = (self.view.scroll_time
                    - f32::from(delta.x) as f64 / self.view.pps)
                    .max(self.model.index.minimum_time());
                self.changed(false, cx);
            }
            Gesture::Column => {
                let m = self.tokens.metrics;
                let width = f32::from(self.bounds.size.width) as f64;
                let x = f32::from(event.position.x - self.bounds.origin.x) as f64;
                self.view.layer_column =
                    x.clamp(m.layer_column as f64 / 2., (width - m.hit as f64).max(1.));
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
        let over = if position.x < left {
            f32::from(position.x - left)
        } else if position.x > right {
            f32::from(position.x - right)
        } else {
            return;
        };
        self.view.scroll_time = (self.view.scroll_time + over as f64 * 0.1 / self.view.pps)
            .max(self.model.index.minimum_time());
        self.changed(false, cx);
    }
    fn marquee(&mut self, cx: &mut Context<Self>) {
        let Gesture::Marquee {
            session,
            origin,
            at,
        } = &mut self.gesture
        else {
            return;
        };
        let (a, b) = (*origin - self.bounds.origin, *at - self.bounds.origin);
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
        let Some(frame) = &self.frame else {
            return;
        };
        let keys = frame.keys_in(area);
        let layers = if self.model.mode == TimelineMode::Tracks {
            frame.clips_in(area)
        } else {
            vec![]
        };
        let plan = session.update(&self.model, keys, layers);
        self.queue.retain(|j| !matches!(j, Job::Apply(_)));
        self.queue.push_back(Job::Apply(plan));
        self.pump(cx);
    }
    fn on_up(&mut self, cx: &mut Context<Self>) -> bool {
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Idle => return false,
            Gesture::Pressed(press) => {
                if let Some(job) = press.release {
                    self.queue.push_back(job);
                }
            }
            Gesture::Dragging { .. } => self.queue.push_back(Job::Finish(true)),
            Gesture::Marquee {
                session,
                origin,
                at,
            } => {
                let delta = at - origin;
                let moved = f32::from(delta.x).abs().max(f32::from(delta.y).abs())
                    >= self.tokens.metrics.drag_threshold;
                if moved {
                    self.queue
                        .push_back(Job::Apply(session.finish(&self.model)));
                } else {
                    self.queue.push_back(Job::Apply(Plan {
                        calls: vec![SelectionCall::Clear.into()],
                        ..Default::default()
                    }));
                }
            }
            Gesture::Column | Gesture::Pan { .. } => self.changed(true, cx),
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
        if event.modifiers.alt || event.modifiers.platform {
            if x >= self.view.layer_column {
                self.zoom_at(x, (dy * WHEEL_ZOOM).exp(), cx);
            }
            return;
        }
        let _ = y;
        let horizontal = dx.abs() > dy.abs() || event.modifiers.shift;
        if horizontal {
            let travel = if dx.abs() > dy.abs() { dx } else { dy };
            self.view.scroll_time = (self.view.scroll_time - travel / self.view.pps)
                .max(self.model.index.minimum_time());
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
        if x < self.view.layer_column {
            let row = self
                .scene
                .as_ref()
                .and_then(|s| s.row_at(y - ruler + self.view.scroll_y))
                .and_then(|r| pm_panel_timeline::v4::parse_row(&r.id).ok());
            if let Some(RowId::Layer(id)) = row {
                self.layer_menu(id, window, cx);
            }
            return;
        }
        let time = self.time_at(x);
        let at = |kind: &str, layer: Option<&str>| {
            vec![(
                "timeline:context",
                serde_json::json!({"kind":kind,"layerId":layer,"time":time}),
            )]
        };
        let context = if y < ruler {
            match frame.hit_op("marker:", x, y, 2.) {
                Some(op) => MenuContext::Marker {
                    id: op.id["marker:".len()..].to_owned(),
                },
                None => MenuContext::Ruler {
                    time: self.time_at(x),
                },
            }
        } else if self.graph_area().is_some_and(|a| y >= a.y) {
            MenuContext::Graph {
                series: self.graph.as_ref().map_or(0, |_| 1),
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
        let mut items = self.model.menu(&context);
        // Graph Fit and Merge have no host. Never offer them live.
        for item in &mut items {
            item.enabled &= !matches!(item.id, "graph.fit" | "graph.merge");
        }
        let ids: Vec<&'static str> = items.iter().map(|i| i.id).collect();
        let rows = items
            .iter()
            .map(|i| serde_json::json!({"id":i.id,"label":pm_panel_timeline::menu_label(i.id),"disabled":!i.enabled,"checked":i.checked}))
            .collect();
        let weak = cx.entity().downgrade();
        self.delegate.clone().extension_menu(
            locations,
            rows,
            window,
            cx,
            Box::new(move |index, cx| {
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
        self.delegate.clone().extension_menu(
            locations,
            crate::layer_commands::layer_menu_rows(),
            window,
            cx,
            Box::new(move |index, cx| {
                let rows = &crate::layer_commands::LAYER_MENU;
                if let Some((command, _)) = index.and_then(|i| rows.get(i)) {
                    delegate.command(command, cx);
                }
            }),
        );
    }
    fn hover(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let (x, y) = self.local(position);
        let m = self.tokens.metrics;
        let cursor =
            if (x - self.view.layer_column).abs() <= m.hit as f64 / 4. && y >= m.ruler as f64 {
                CursorStyle::ResizeColumn
            } else if let Some((_, Some(_))) = self.frame.as_ref().and_then(|f| f.hit_clip(x, y)) {
                CursorStyle::ResizeLeftRight
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
        self.graph_area()
            .and_then(|a| self.graph_ops(a))
            .unwrap_or_default()
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
    pub fn model(&self) -> Arc<TimelineModel> {
        self.model.clone()
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
            client.begin_drag(kind).await.map_err(text)?;
            Ok(vec![])
        }
        Job::Sample(sample) => {
            if client.active_drag().is_some() {
                client.drag(sample).await.map_err(text)?;
            }
            Ok(vec![])
        }
        Job::Finish(commit) => {
            if client.active_drag().is_some() {
                client.finish_drag(commit).await.map_err(text)?;
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
    Ok(match spec {
        Spec::Layers => DragKind::Layers { ids },
        Spec::Trim(Edge::In) => DragKind::TrimIn { ids },
        Spec::Trim(Edge::Out) => DragKind::TrimOut { ids },
        Spec::Offset => DragKind::QuickOffsetLayers { ids },
        Spec::Keys => DragKind::Keys { ids: keys },
        Spec::GraphKeys => DragKind::GraphKeys { ids: keys },
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
}
impl ModelPresenter for Presenter {
    fn accessibility_label(&self, row: &RowId) -> String {
        match row {
            RowId::Layer(id) => id.clone(),
            RowId::Property { channel, .. } => channel.clone(),
        }
    }
    fn layer_label(&self, layer: &pm_panel_timeline::Layer) -> Label {
        match layer.kind.as_str() {
            "text" | "shape" => Label::Blue,
            "video" | "image" | "sequence" => Label::Teal,
            "audio" => Label::Green,
            "solid" => Label::Orange,
            "adjustment" | "effect" => Label::Violet,
            _ => Label::Gray,
        }
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
    fn interpolation(&self, key: &pm_panel_timeline::Key) -> Interpolation {
        match key.stored.get("e").and_then(pm_model::Value::as_str) {
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
fn build(
    client: &TimelineClient,
    tokens: &Tokens,
    offline: Arc<BTreeSet<String>>,
) -> Option<Built> {
    let published: Arc<Published> = client.kernel.published(&client.project)?;
    let presenter = Presenter { offline };
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
    })
}
/// The frame the canvas prepared, and where, for the clips' accessible elements.
type Shown = Rc<RefCell<Option<(Point<Pixels>, Arc<Frame>)>>>;
impl Render for TimelineSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let tokens = self.tokens.clone();
        let graph = self.graph_area().and_then(|area| self.graph_ops(area));
        let decorations = self.decorations.clone();
        let marquee = match &self.gesture {
            Gesture::Marquee { origin, at, .. } => Some(Bounds::from_corners(
                point(origin.x.min(at.x), origin.y.min(at.y)),
                point(origin.x.max(at.x), origin.y.max(at.y)),
            )),
            _ => None,
        };
        let weak = this.clone();
        let t = &self.tokens;
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
            .aria_label("Timeline tracks")
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
                            .update(cx, |s, cx| s.prepared(bounds, cx))
                            .ok()
                            .flatten();
                        *prepared.borrow_mut() = frame.clone().map(|f| (bounds.origin, f));
                        frame
                    },
                    move |bounds, frame, window, cx| {
                        if let Some(frame) = frame {
                            // The outline row owns the keyframe toggle; the scene's
                            // copy would sit stale beneath it.
                            for op in frame.ops.iter().filter(|op| !op.id.starts_with("toggle:")) {
                                paint(op, bounds.origin, &tokens, window, cx);
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
                        if let Some(area) = marquee {
                            window.paint_quad(
                                PaintQuad::new(area)
                                    .border_widths(1f32)
                                    .border_color(rgb(tokens.colors.secondary)),
                            );
                        }
                        let moved = this.clone();
                        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ = moved.update(cx, |s, cx| {
                                    if s.on_move(e, cx) {
                                        cx.stop_propagation();
                                    } else if s.bounds.contains(&e.position) {
                                        s.hover(e.position, cx);
                                    }
                                });
                            }
                        });
                        let released = this.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                let _ = released.update(cx, |s, cx| {
                                    s.on_up(cx);
                                });
                            }
                        });
                    },
                )
                .size_full(),
            )
            // Over the canvas.
            .children(tip)
    }
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
        "rect" | "rounded-rect" | "work-area-handle" | "square" => {
            window.paint_quad(
                PaintQuad::new(area)
                    .background(color)
                    .corner_radii(px(op.radius as f32)),
            );
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
            window.paint_quad(
                PaintQuad::new(area)
                    .background(color)
                    .corner_radii(area.size.width / 2.),
            );
        }
        "diamond" | "diamond.fill" | "diamond.lefthalf.filled" | "keyframe.hold" | "marker" => {
            diamond(area, color, window)
        }
        // The playhead pin: a rounded top and a rounder point (the owner's demo).
        "ph-head" => {
            let r = |key: &str| px(op.detail[key].as_f64().unwrap_or(0.) as f32);
            window.paint_quad(
                PaintQuad::new(area)
                    .background(color)
                    .corner_radii(Corners {
                        top_left: r("topRadius"),
                        top_right: r("topRadius"),
                        bottom_right: r("bottomRadius"),
                        bottom_left: r("bottomRadius"),
                    }),
            );
        }
        "line" => {
            if let (Some(from), Some(to)) =
                (point_of(&op.detail["from"]), point_of(&op.detail["to"]))
            {
                let mut path = PathBuilder::stroke(px(op.stroke as f32));
                path.move_to(at(from.0, from.1));
                path.line_to(at(to.0, to.1));
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
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
                    path.line_to(at(to.0, to.1));
                }
            }
            if started && let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        }
        "text" | "waveform" => {
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
                    line = shape(fitted, window);
                }
            }
            let height = font_size * t.typography.dense_leading;
            let y = b.y + (b.height - f32::from(height) as f64) / 2.;
            let _ = line.paint(
                at(b.x, y.max(b.y)),
                height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
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
    let mut path = PathBuilder::fill();
    path.move_to(point(c.x, c.y - h));
    path.line_to(point(c.x + w, c.y));
    path.line_to(point(c.x, c.y + h));
    path.line_to(point(c.x - w, c.y));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
