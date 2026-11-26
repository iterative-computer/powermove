//! Whole-workspace cases (PM-698): the real pm-shell editor docking the
//! built-in panels as the Design workspace places them (Colour under the
//! Project / Effect Controls tabs), every realm running on one kernel, a selected layer
//! with an animated position and an eight-effect stack, and a playback
//! driver: a real engine viewer, offscreen, playing on the 120 Hz cadence
//! while the main thread does what the display link asks of it each tick
//! (`playback.rs`: the viewer's tick and status, Properties following the
//! frame through `follow_playback`'s gate, the timeline paging after the
//! head). Every interaction runs paused and again with playback presenting.
//!
//! Panel bodies mirror `Controller::builtin_panel` (a side panel grows with
//! its content inside a scrolling body; the viewer and timeline fill their
//! slot); what needs the Controller itself (facts, extensions, AppKit
//! controls, CALayers) is not here.
use super::*;
use crate::timeline_surface::TimelineSurface;
use crate::timeline_tests::{Recorder, Recording};
use gpui::{
    AnyElement, InteractiveElement, IntoElement, MouseMoveEvent, ParentElement, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, TouchPhase, div, size,
};
use pm_builtins::{Builtins, Publication};
use pm_shell::view::Editor;
use std::{collections::HashMap, sync::Arc};

/// Realms running (the Design workspace's panels plus Colour).
const MOUNTED: [&str; 8] = [
    "inspector",
    "layer-effects",
    "color",
    "fxbrowser",
    "easing",
    "assets",
    "timeline",
    "viewer",
];
/// Panels that fill their slot and own their scroll (`builtin_panel`).
const FIXED: [&str; 2] = ["viewer", "timeline"];
/// The effect stack on the selected layer.
const EFFECTS: [&str; 8] = [
    "blur",
    "glow",
    "brightnessContrast",
    "levels",
    "shadow",
    "sharpen",
    "stroke",
    "exposure",
];
const VSYNC: Duration = Duration::from_nanos(8_333_333);

// ------------------------------------------------------------- the dock

/// The shell's integration: built-in panels only, bodies as the app builds
/// them.
struct Dock {
    runtime: Arc<Builtins>,
    tokens: pm_design::Tokens,
    natives: Natives,
    store: Rc<pm_shell::workspace::Store>,
    pending: RefCell<HashMap<String, Publication>>,
    views: RefCell<HashMap<String, Entity<SurfaceView>>>,
    scrolls: RefCell<HashMap<String, ScrollHandle>>,
}

impl pm_shell::integration::Integration for Dock {
    fn observe(&self, _: &mut gpui::Context<Editor>) {}
    fn command(&self, _: &pm_shell::Command, _: &mut gpui::Window, _: &mut App) -> bool {
        false
    }
    fn workspaces(&self, _: &App) -> Option<Rc<pm_shell::workspace::Store>> {
        Some(self.store.clone())
    }
    fn project_id(&self, _: &App) -> Option<String> {
        Some(PROJECT.into())
    }
    fn status(&self, _: &App) -> String {
        String::new()
    }
    fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    fn paints_own_ground(&self, id: &str, _: &App) -> bool {
        id == "timeline"
    }
    fn natural_panel_height(&self, id: &str, cx: &App) -> Option<f32> {
        let height = self.views.borrow().get(id)?.read(cx).natural_body_height();
        (height > 0.).then_some(height + self.tokens.metrics.header)
    }
    fn panel(
        &self,
        id: &str,
        rect: pm_shell::layout::Rect,
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        if let Some(first) = self.pending.borrow_mut().remove(id) {
            let view = cx.new(|cx| {
                SurfaceView::new(
                    self.runtime.clone(),
                    first,
                    self.tokens.clone(),
                    self.natives.clone(),
                    window,
                    cx,
                )
            });
            if !FIXED.contains(&id) {
                view.update(cx, |v, _| v.grows = true);
            }
            if id == "timeline" {
                self.natives.outline.bind(view.downgrade());
            }
            self.views.borrow_mut().insert(id.into(), view);
        }
        let view = self.views.borrow().get(id).cloned()?;
        view.update(cx, |v, cx| v.attach(window, cx));
        let body = (rect.h - self.tokens.metrics.header).max(0.);
        if FIXED.contains(&id) {
            return Some(
                div()
                    .w_full()
                    .h(px(body))
                    .overflow_hidden()
                    .child(crate::builtins::fixed_body_view(id, &view))
                    .into_any_element(),
            );
        }
        let handle = self
            .scrolls
            .borrow_mut()
            .entry(id.into())
            .or_default()
            .clone();
        view.update(cx, |v, _| v.scrolls_in(&handle));
        let content = view.read(cx).in_body(body);
        Some(
            div()
                .relative()
                .w_full()
                .h(px(body))
                .child(
                    div()
                        .id(SharedString::from(format!("panel-body-{id}")))
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&handle)
                        .scroll_edge_fade_y(px(pm_design::DOCK_SCROLL_FADE))
                        .child(view.clone().cached(content)),
                )
                .into_any_element(),
        )
    }
}

// -------------------------------------------------------- the workspace

struct Workspace {
    /// Publications that reached each view since the last case.
    published: std::collections::BTreeMap<String, usize>,
    /// Timeline row bounds as first painted (a cached body replays its
    /// drawing and leaves no debug bounds).
    rows: HashMap<String, Bounds<Pixels>>,
    rig: Rig,
    dock: Rc<Dock>,
    editor: Entity<Editor>,
    timeline: Entity<TimelineSurface>,
    vcx: VisualTestContext,
}

/// `project_json` with `l0`'s position animated across the comp (Properties
/// follows it while playing) and its effect stack added by the kernel.
fn workspace_project(n: usize) -> String {
    let mut project: Value = serde_json::from_str(&project_json(n)).unwrap();
    project["layers"][0]["p"]["position.x"]["kf"] = json!([{"t":0,"v":100},{"t":10,"v":500}]);
    // Two minutes, so the timeline has time to scroll sideways through.
    project["dur"] = json!(120);
    // Offline stills: the Project panel lists them (rows to hover and scroll).
    project["assets"] = (0..40)
        .map(|i| {
            let id = format!("a{i}");
            (
                id.clone(),
                json!({"id": id, "kind": "image", "name": format!("Still {i}.png")}),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into();
    project.to_string()
}

fn add_effects(kernel: &pm_kernel::KernelHandle) {
    use pm_kernel::commands::effects::{AddEffect, EffectCommand, saved_builtin_definition};
    let p = published(kernel);
    let commands = EFFECTS
        .iter()
        .enumerate()
        .filter_map(|(i, kind)| {
            let definition =
                saved_builtin_definition(&p.snapshot.document, kind, None, 640., 360.).ok()?;
            Some(
                EffectCommand::Add(Box::new(AddEffect {
                    layer_id: "l0".into(),
                    effect_id: format!("fx{i}"),
                    definition,
                    parameters: Default::default(),
                    inputs: None,
                    open: true,
                    enabled: true,
                    enabled_animation: None,
                    index: None,
                }))
                .into(),
            )
        })
        .collect();
    call_now(
        kernel,
        Call::Transact {
            label: Some("Add Effect".into()),
            expected_revision: None,
            body: WriteBody::Commands(commands),
        },
    );
}

/// The Design workspace, with Colour under Project / Effect Controls.
fn layout_record() -> Value {
    let mut record = pm_shell::workspace::preset("design").unwrap();
    let docks = record["layout"]["docks"].as_array_mut().unwrap();
    let left = docks.iter_mut().find(|d| d["id"] == "left").unwrap();
    left["panels"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "color", "size": 160}));
    // Effects in a slot its list overflows, as on a laptop screen: its
    // scroll cases must move something.
    let right = docks.iter_mut().find(|d| d["id"] == "right").unwrap();
    for panel in right["panels"].as_array_mut().unwrap() {
        if panel["id"] == "fxbrowser" {
            panel["size"] = json!(200);
        }
    }
    record["id"] = json!("ui-latency");
    record["builtin"] = json!(false);
    record["scope"] = json!("project");
    record["projectId"] = json!(PROJECT);
    record
}

fn workspace(cx: &mut TestAppContext) -> Result<Workspace, String> {
    let rig = rig_project("inspector", &workspace_project(layer_count()));
    add_effects(&rig.kernel);
    let mut rig = rig;
    let runtime = Arc::get_mut(&mut rig.runtime).ok_or("the panel runtime is shared")?;
    for panel in &MOUNTED[1..] {
        let ticket = runtime.mount(panel).map_err(|e| format!("{panel}: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            if let Some(result) = ticket.try_take() {
                result.map_err(|e| format!("{panel}: {e}"))?;
                break;
            }
            if Instant::now() > deadline {
                return Err(format!("{panel} did not mount"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    select_now(&rig.kernel, &["l0"]);
    for (_, ticket) in rig.runtime.publish() {
        if let Ok(ticket) = ticket {
            let deadline = Instant::now() + Duration::from_secs(20);
            while ticket.try_take().is_none() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    let pending: HashMap<String, Publication> = rig
        .runtime
        .take_publications()
        .into_iter()
        .map(|p| (p.panel.clone(), p))
        .collect();
    for panel in MOUNTED {
        if !pending.contains_key(panel) {
            return Err(format!("{panel} did not publish"));
        }
    }
    let p = published(&rig.kernel);
    let shell_tokens = tokens();
    let timeline_tokens =
        crate::builtins::timeline_chrome(shell_tokens.clone(), &p.workspace, &Value::Null);
    let natives = Natives::default();
    let recorder = Rc::new(Recorder::default());
    recorder.runtime.replace(Some(rig.runtime.clone()));
    recorder.published.replace(Some(rig.kernel.clone()));
    let timeline = cx.new(|cx| {
        let mut t = TimelineSurface::new(
            rig.kernel.clone(),
            PROJECT.into(),
            timeline_tokens,
            Rc::new(Recording(recorder)),
            natives.outline.clone(),
            Default::default(),
            cx,
        );
        t.set_outline_runtime(&rig.runtime);
        t
    });
    natives
        .views
        .borrow_mut()
        .insert("timeline".into(), timeline.clone().into());
    let record = layout_record();
    let dock = Rc::new(Dock {
        runtime: rig.runtime.clone(),
        tokens: shell_tokens.clone(),
        natives,
        store: Rc::new(pm_shell::workspace::Store {
            records: pm_shell::workspace::seeded(&[]),
            project: Some(record),
            project_id: Some(PROJECT.into()),
            ..Default::default()
        }),
        pending: RefCell::new(pending),
        views: RefCell::default(),
        scrolls: RefCell::default(),
    });
    cx.executor().allow_parking();
    let integration = dock.clone();
    let (editor, vcx) = cx.add_window_view(|_, cx| {
        let mut editor = Editor::new(shell_tokens, None, cx);
        editor.integration = Some(integration);
        editor
    });
    let mut vcx = vcx.clone();
    vcx.update(|window, _| window.activate());
    vcx.simulate_resize(size(px(1728.), px(1080.)));
    let mut ws = Workspace {
        published: Default::default(),
        rows: HashMap::new(),
        rig,
        dock,
        editor,
        timeline,
        vcx,
    };
    let mut warm = Probe::start();
    for _ in 0..3 {
        ws.settle(&mut warm, None);
        draw(&mut ws.vcx, &mut warm);
    }
    // Cached views reuse their paint and record no debug bounds, and a
    // panel whose height changed draws again on its own, not the whole
    // window (PM-696): once those frames settle, one full paint records the
    // timeline's rows, read before any frame draws from the caches again.
    for _ in 0..8 {
        let callbacks = ws.vcx.update(|window, cx| window.simulate_next_frame(cx));
        ws.vcx.run_until_parked();
        if callbacks == 0 {
            break;
        }
    }
    ws.vcx.update(|window, _| window.refresh());
    ws.vcx.run_until_parked();
    for i in 0..layer_count() {
        let id = format!("l{i}");
        let selector = Box::leak(format!(r#"timeline:row:["layer","{id}"]"#).into_boxed_str());
        if let Some(b) = ws.vcx.debug_bounds(selector) {
            ws.rows.insert(id, b);
        }
    }
    for panel in ["inspector", "fxbrowser", "assets", "timeline", "viewer"] {
        if !ws.dock.views.borrow().contains_key(panel) {
            return Err(format!("{panel} was not drawn"));
        }
    }
    Ok(ws)
}

impl Workspace {
    fn views(&self) -> Vec<Entity<SurfaceView>> {
        self.dock.views.borrow().values().cloned().collect()
    }
    /// GPUI work, panels and timeline settled; the playback driver keeps
    /// its cadence meanwhile.
    fn settle(&mut self, probe: &mut Probe, mut playback: Option<&mut Playback>) -> bool {
        loop {
            if let Some(p) = playback.as_deref_mut() {
                p.catch_up(self, probe);
            }
            probe.run(|| self.vcx.run_until_parked());
            let views = self.views();
            let timeline = self.timeline.clone();
            let idle = probe.run(|| {
                views
                    .iter()
                    .all(|v| v.read_with(&self.vcx, |v, _| v.idle()))
                    && timeline.read_with(&self.vcx, |t, _| t.idle())
            });
            if idle {
                return true;
            }
            if probe.start.elapsed() > WAIT {
                return false;
            }
            std::thread::sleep(POLL);
        }
    }
    /// `drain_builtins`: each publication to its view (or kept for a panel
    /// not drawn yet). Returns how many reached a view.
    fn drain(&mut self, probe: &mut Probe) -> usize {
        let mut n = 0;
        for p in probe.run(|| self.rig.runtime.take_publications()) {
            *self.published.entry(p.panel.clone()).or_default() += 1;
            let view = self.dock.views.borrow().get(&p.panel).cloned();
            match view {
                Some(view) => {
                    probe.run(|| view.update(&mut self.vcx, |v, cx| v.publish(p, cx)));
                    n += 1;
                }
                None => {
                    self.dock.pending.borrow_mut().insert(p.panel.clone(), p);
                }
            }
        }
        let _ = self.rig.runtime.take_errors();
        n
    }
    /// `Controller::refresh` after a kernel publication: the timeline syncs,
    /// every realm publishes (`update_builtins`), the views take their trees.
    fn refresh(&mut self, probe: &mut Probe, mut playback: Option<&mut Playback>) -> usize {
        probe.run(|| self.timeline.update(&mut self.vcx, |t, cx| t.published(cx)));
        let tickets = probe.run(|| self.rig.runtime.publish());
        for (_, ticket) in tickets {
            let Ok(ticket) = ticket else { continue };
            loop {
                if let Some(p) = playback.as_deref_mut() {
                    p.catch_up(self, probe);
                }
                probe.run(|| self.vcx.run_until_parked());
                if ticket.try_take().is_some() || probe.start.elapsed() > WAIT {
                    break;
                }
                std::thread::sleep(POLL);
            }
        }
        self.drain(probe)
    }
    fn rect(&self, id: &str) -> Option<pm_shell::layout::Rect> {
        self.editor.read_with(&self.vcx, |e, _| e.panel_rect(id))
    }
    fn click(&mut self, probe: &mut Probe, at: Point<Pixels>) {
        click_center(&mut self.vcx, probe, at);
    }
    /// Shows a tab's panel (a click on its tab), untimed. Project and
    /// Effect Controls share the left dock's tab group.
    fn show_tab(&mut self, id: &str) {
        if !matches!(id, "assets" | "layer-effects") {
            return;
        }
        let Some(tab) = self.editor.read_with(&self.vcx, |e, _| e.tab_rect(id)) else {
            return;
        };
        let mut probe = Probe::start();
        self.click(
            &mut probe,
            point(px(tab.x + tab.w / 2.), px(tab.y + tab.h / 2.)),
        );
        self.settle(&mut probe, None);
        draw(&mut self.vcx, &mut probe);
    }
}

// ---------------------------------------------------- playback driver

/// A real engine viewer playing offscreen, ticked on the display's
/// cadence from the main thread, as the display link does.
struct Playback {
    _engine: pm_engine_service::EngineService,
    viewer: pm_engine_service::Viewer,
    rate: pm_time::Rate,
    next: Instant,
    followed: Option<i64>,
    followed_at: Option<Instant>,
    follow: Option<Box<dyn Fn() -> bool>>,
    ticks: usize,
    presented: usize,
    last_frame: Option<i64>,
    last_status: String,
    follows: usize,
    tick_ms: Vec<f64>,
}

impl Playback {
    fn start(ws: &Workspace) -> Result<Self, String> {
        use pm_engine_service::{EngineService, Output, ViewerConfig};
        use pm_protocol::{clock::TransportCommand, common::PlaybackRate};
        let gpu = pm_gpu::Gpu::new("ui-latency playback").map_err(|e| e.to_string())?;
        let engine = EngineService::start(gpu).map_err(|e| e.to_string())?;
        let p = published(&ws.rig.kernel);
        let rate = crate::session::published_rate(&p);
        let viewer = engine
            .viewer(ViewerConfig {
                composition: p.snapshot.render_comp(),
                snapshot: p.snapshot.clone(),
                media: Default::default(),
                output: Output {
                    width: 1280,
                    height: 720,
                    display: 0,
                    headroom: 1.,
                    crop: None,
                },
                layer: None,
                rate,
                end: None,
                audio: false,
            })
            .map_err(|e| e.to_string())?;
        viewer
            .resources(Arc::new(pm_engine_service::resources::ResourceSnapshot {
                generation: 1,
                ..Default::default()
            }))
            .map_err(|e| e.to_string())?;
        viewer
            .scrub(pm_protocol::common::Ticks::ZERO)
            .map_err(|e| e.to_string())?;
        viewer
            .command(TransportCommand::Play {
                rate: PlaybackRate::NORMAL,
            })
            .map_err(|e| e.to_string())?;
        let mut playback = Self {
            _engine: engine,
            viewer,
            rate,
            next: Instant::now(),
            followed: None,
            followed_at: None,
            follow: None,
            ticks: 0,
            presented: 0,
            last_frame: None,
            last_status: String::new(),
            follows: 0,
            tick_ms: vec![],
        };
        // Until frames present.
        let deadline = Instant::now() + Duration::from_secs(20);
        while playback.presented < 3 && Instant::now() < deadline {
            playback.tick_now();
            std::thread::sleep(VSYNC);
        }
        if playback.presented < 3 {
            return Err(format!(
                "the engine presented {} frames in 20 s ({})",
                playback.presented, playback.last_status
            ));
        }
        Ok(playback)
    }
    /// The viewer's part of one display-link tick; the playhead time.
    fn tick_now(&mut self) -> Option<f64> {
        let _ = self
            .viewer
            .display_tick(pm_engine_service::trace::now() + 1. / 120., 1. / 120.);
        let status = self.viewer.status();
        // Offscreen, a frame is shown once it is done: a new frame time.
        if let Some(f) = &status.frame
            && f.gpu_ok
            && Some(f.time.get()) != self.last_frame
        {
            self.last_frame = Some(f.time.get());
            self.presented += 1;
        }
        self.last_status = format!(
            "frame {:?}, clock {:?}, error {:?}",
            status.frame.as_ref().map(|f| (f.time.get(), f.gpu_ok)),
            status.clock.as_ref().map(|c| c.state),
            status.error
        );
        status
            .clock
            .map(|c| c.media_time.get() as f64 / pm_protocol::common::TICKS_PER_SECOND as f64)
    }
    /// Every tick due by now, as the display link delivers them.
    fn catch_up(&mut self, ws: &mut Workspace, probe: &mut Probe) {
        while Instant::now() >= self.next {
            self.next += VSYNC;
            if Instant::now() > self.next + VSYNC * 4 {
                // Fell far behind (a long blocking step): the link drops ticks.
                self.next = Instant::now() + VSYNC;
            }
            self.ticks += 1;
            let at = Instant::now();
            let time = probe.run(|| self.tick_now());
            // `follow_playback`: Properties takes the frame on its gate.
            if self.follow.as_ref().is_some_and(|done| done()) {
                self.follow = None;
                ws.drain(probe);
            }
            if let Some(time) = time
                && let Some(frame) = crate::session::frame_start_at(self.rate, time)
                && self.follow.is_none()
                && self.followed != Some(frame)
                && crate::builtins::follows_selection(Some(&published(&ws.rig.kernel)))
                && crate::builtins::follow_due(
                    self.followed,
                    self.followed_at,
                    frame,
                    Instant::now(),
                )
            {
                self.followed = Some(frame);
                self.followed_at = Some(Instant::now());
                let seconds = frame as f64 / pm_protocol::common::TICKS_PER_SECOND as f64;
                if let Some(Ok(ticket)) =
                    probe.run(|| ws.rig.runtime.follow_time("inspector", seconds, true))
                {
                    self.follows += 1;
                    self.follow = Some(Box::new(move || ticket.try_take().is_some()));
                }
            }
            // The timeline pages to keep its head in sight.
            if let Some(time) = time {
                probe.run(|| {
                    ws.timeline
                        .update(&mut ws.vcx, |t, cx| t.follow_playhead(time, cx))
                });
            }
            self.tick_ms.push(at.elapsed().as_secs_f64() * 1e3);
        }
    }
    /// Runs the display link until the next tick, untimed for the case:
    /// one input per vsync, as a wheel or trackpad delivers them.
    fn pace(&mut self, ws: &mut Workspace) {
        let mut probe = Probe::start();
        let until = self.next;
        while Instant::now() < until {
            std::thread::sleep(POLL);
        }
        self.catch_up(ws, &mut probe);
        probe.run(|| ws.vcx.run_until_parked());
        if self.follow.is_none() {
            draw(&mut ws.vcx, &mut probe);
        }
    }
    fn note(&mut self, case: &mut Case) {
        case.note(
            "playback",
            json!({"ticks": self.ticks, "frames_presented": self.presented,
                "properties_follows": self.follows}),
        );
        case.add_extra_ms("playback_tick_main_ms", &self.tick_ms);
        self.ticks = 0;
        self.presented = 0;
        self.follows = 0;
        self.tick_ms.clear();
    }
}

// ----------------------------------------------------------------- cases

/// One timed iteration around `input`: the result drawn, playback ticking
/// meanwhile when it runs.
fn step(
    ws: &mut Workspace,
    playback: &mut Option<Playback>,
    input: impl FnOnce(&mut Workspace, &mut Probe),
    publishes: bool,
) -> Sample {
    let mut probe = Probe::start();
    input(ws, &mut probe);
    let mut ok = ws.settle(&mut probe, playback.as_mut());
    if publishes {
        // What it shows is the step's to check (the selection, say): the
        // panels' trees may reach the views through the playback's drain, and
        // a panel whose tree did not change publishes nothing.
        ws.refresh(&mut probe, playback.as_mut());
        ok &= ws.settle(&mut probe, playback.as_mut());
    } else {
        ws.drain(&mut probe);
    }
    draw(&mut ws.vcx, &mut probe);
    let sample = probe.sample(!ok);
    if let Some(p) = playback.as_mut() {
        p.pace(ws);
    }
    sample
}

fn wheel(
    at: Point<Pixels>,
    d: Point<Pixels>,
    touch: TouchPhase,
    momentum: bool,
) -> ScrollWheelEvent {
    ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(d),
        modifiers: Modifiers::none(),
        touch_phase: touch,
        is_momentum: momentum,
    }
}

/// A trackpad flick: the fingers' moves, then momentum decaying to rest.
fn momentum_events(at: Point<Pixels>, horizontal: bool, sign: f32) -> Vec<ScrollWheelEvent> {
    let d = |v: f32| {
        if horizontal {
            point(px(v * sign), px(0.))
        } else {
            point(px(0.), px(v * sign))
        }
    };
    let mut out = vec![wheel(at, d(0.), TouchPhase::Started, false)];
    out.extend([6., 14., 22., 26., 24.].map(|v| wheel(at, d(-v), TouchPhase::Moved, false)));
    out.push(wheel(at, d(0.), TouchPhase::Ended, false));
    let mut v = 40f32;
    while v > 1. {
        out.push(wheel(at, d(-v), TouchPhase::Moved, true));
        v *= 0.82;
    }
    out.push(wheel(at, d(0.), TouchPhase::Ended, true));
    out
}

/// A point of the panel's body where a wheel step moves it (outside any
/// measured step: one step and one back). Its middle can hold a control that
/// takes the wheel for itself (a colour wheel), or nothing that scrolls.
fn scroll_target(ws: &mut Workspace, panel: &str, horizontal: bool) -> Option<Point<Pixels>> {
    let r = ws.rect(panel)?;
    let header = ws.dock.tokens.metrics.header;
    for (fx, fy) in [
        (0.5, 0.5),
        (0.5, 0.3),
        (0.5, 0.75),
        (0.15, 0.5),
        (0.85, 0.6),
    ] {
        let at = point(px(r.x + r.w * fx), px(r.y + header + (r.h - header) * fy));
        let before = scroll_position(ws, panel, horizontal);
        let mut moved = false;
        for v in [-40., 40.] {
            let d = if horizontal {
                point(px(v), px(0.))
            } else {
                point(px(0.), px(v))
            };
            ws.vcx
                .simulate_event(wheel(at, d, TouchPhase::Moved, false));
            ws.vcx.run_until_parked();
            moved |= (scroll_position(ws, panel, horizontal) - before).abs() > 1e-3;
        }
        if moved {
            return Some(at);
        }
    }
    None
}

/// Where a scroll case's target stands: the timeline's scroll (rows or
/// time), or a side panel's body and its own scrollers.
fn scroll_position(ws: &Workspace, panel: &str, horizontal: bool) -> f64 {
    if panel == "timeline" {
        return ws.timeline.read_with(&ws.vcx, |t, _| {
            if horizontal {
                t.scroll_time()
            } else {
                t.scroll_y()
            }
        });
    }
    let body = ws
        .dock
        .scrolls
        .borrow()
        .get(panel)
        .map_or(0., |h| f64::from(f32::from(h.offset().y)).abs());
    let inner = ws.dock.views.borrow().get(panel).map_or(0., |v| {
        f64::from(v.read_with(&ws.vcx, |v, _| v.scroll_offsets()))
    });
    // A panel that grows with its content scrolls its dock's column.
    let column = ws.editor.read_with(&ws.vcx, |e, _| {
        let (dock, _, _) = e.layout.find(panel)?;
        Some(f64::from(e.dock_scroll_offset(&e.layout.docks[dock].id)))
    });
    body + inner + column.unwrap_or(0.)
}

fn scroll_case(
    ws: &mut Workspace,
    playback: &mut Option<Playback>,
    id: &str,
    name: &str,
    panel: &str,
    horizontal: bool,
    momentum: bool,
) -> Case {
    let mut case = Case::new(id, "workspace", name);
    let Some(at) = scroll_target(ws, panel, horizontal) else {
        return case.skip(format!("no point of {panel} scrolls"));
    };
    let events: Vec<ScrollWheelEvent> = if momentum {
        let mut e = momentum_events(at, horizontal, 1.);
        e.extend(momentum_events(at, horizontal, -1.));
        e
    } else {
        (0..iterations().max(16))
            .map(|k| {
                let v = if (k / 4) % 2 == 0 { -40. } else { 40. };
                let d = if horizontal {
                    point(px(v), px(0.))
                } else {
                    point(px(0.), px(v))
                };
                wheel(at, d, TouchPhase::Moved, false)
            })
            .collect()
    };
    // The target must move: a swallowed gesture or a body with nothing to
    // scroll would measure less work and pass (PM-698 review).
    let (mut least, mut most) = (f64::MAX, f64::MIN);
    for event in events {
        let sample = step(
            ws,
            playback,
            |ws, probe| probe.run(|| ws.vcx.simulate_event(event)),
            false,
        );
        case.samples.push(sample);
        let at = scroll_position(ws, panel, horizontal);
        (least, most) = (least.min(at), most.max(at));
    }
    case.note("moved", json!(most - least));
    if most - least < 1e-3 {
        return case.skip(format!("the gesture did not move {panel}"));
    }
    case
}

/// Pointer moves across up to 16 painted rows or controls of a panel.
fn hover_case(
    ws: &mut Workspace,
    playback: &mut Option<Playback>,
    id: &str,
    name: &str,
    panel: &str,
) -> Case {
    let mut case = Case::new(id, "workspace", name);
    let mut spots = vec![];
    if panel == "timeline" {
        for i in 0..12 {
            if let Some(b) = ws.rows.get(&format!("l{i}")) {
                spots.push(b.center());
            }
        }
    } else if let Some(view) = ws.dock.views.borrow().get(panel).cloned() {
        // A fresh paint of every view, outside any measured step: a cached
        // panel replays its paint and leaves no bounds to aim at.
        ws.vcx.update(|window, _| window.refresh());
        ws.vcx.run_until_parked();
        let ids = nodes(&view, &ws.vcx, |n| {
            matches!(
                role(n),
                "button"
                    | "tab"
                    | "option"
                    | "menuitem"
                    | "row"
                    | "listitem"
                    | "treeitem"
                    | "cell"
                    | "gridcell"
            )
        });
        let body = ws.rect(panel);
        for node in ids {
            if let Some(b) = painted(&view, &mut ws.vcx, node)
                && body.is_some_and(|r| {
                    let c = b.center();
                    f32::from(c.y) > r.y && f32::from(c.y) < r.y + r.h
                })
            {
                spots.push(b.center());
            }
            if spots.len() >= 16 {
                break;
            }
        }
    }
    if spots.len() < 2 {
        return case.skip("fewer than two painted rows");
    }
    for k in 0..iterations().max(16) {
        let at = spots[k % spots.len()];
        let sample = step(
            ws,
            playback,
            |ws, probe| probe.run(|| ws.vcx.simulate_mouse_move(at, None, Modifiers::none())),
            false,
        );
        case.samples.push(sample);
    }
    case.note("rows", json!(spots.len()));
    case
}

/// A click on a layer's timeline row with every panel open: the kernel
/// selects, every realm publishes, every view takes its tree, one frame.
fn select_case(ws: &mut Workspace, playback: &mut Option<Playback>, id: &str, name: &str) -> Case {
    let mut case = Case::new(id, "workspace", name).priority();
    for k in 0..iterations() {
        let layer = if k % 2 == 0 { "l1" } else { "l0" };
        let Some(b) = ws.rows.get(layer).copied() else {
            return case.skip("timeline rows were not painted");
        };
        let at = point(b.origin.x + px(12.), b.center().y);
        let kernel = ws.rig.kernel.clone();
        let mut selected = true;
        let sample = step(
            ws,
            playback,
            |ws, probe| {
                ws.click(probe, at);
                selected = pump_until(&ws.vcx, probe, |_| {
                    published(&kernel).selection.layers() == [layer.to_owned()]
                });
            },
            true,
        );
        case.samples.push(Sample {
            missed: sample.missed || !selected,
            ..sample
        });
    }
    // Leave l0 selected for the cases after.
    select_now(&ws.rig.kernel, &["l0"]);
    let mut probe = Probe::start();
    ws.refresh(&mut probe, None);
    ws.settle(&mut probe, None);
    draw(&mut ws.vcx, &mut probe);
    case
}

fn tabs_case(ws: &mut Workspace, playback: &mut Option<Playback>, id: &str, name: &str) -> Case {
    let mut case = Case::new(id, "workspace", name);
    for k in 0..iterations() {
        let tab = if k % 2 == 0 {
            "assets"
        } else {
            "layer-effects"
        };
        let Some(r) = ws.editor.read_with(&ws.vcx, |e, _| e.tab_rect(tab)) else {
            return case.skip(format!("{tab} has no tab"));
        };
        let at = point(px(r.x + r.w / 2.), px(r.y + r.h / 2.));
        let sample = step(ws, playback, |ws, probe| ws.click(probe, at), false);
        let shown = ws.editor.read_with(&ws.vcx, |e, _| {
            e.layout
                .find(tab)
                .is_some_and(|(d, g, i)| e.layout.docks[d].groups[g].active == i)
        });
        case.samples.push(Sample {
            missed: sample.missed || !shown,
            ..sample
        });
    }
    case
}

/// Window ▸ Effects off and on (`panel:toggle:`), as the menu does it.
fn toggle_case(ws: &mut Workspace, playback: &mut Option<Playback>, id: &str, name: &str) -> Case {
    let mut case = Case::new(id, "workspace", name);
    // An even count: Effects is open again for the cases after.
    for _ in 0..iterations().next_multiple_of(2) {
        let editor = ws.editor.clone();
        let mut toggled = false;
        let sample = step(
            ws,
            playback,
            |ws, probe| {
                toggled = probe.run(|| {
                    editor.update(&mut ws.vcx, |e, cx| {
                        e.run_shell_command("panel:toggle:fxbrowser", cx)
                    })
                });
            },
            false,
        );
        case.samples.push(Sample {
            missed: sample.missed || !toggled,
            ..sample
        });
    }
    case
}

/// A drag of the right dock's edge, per pointer move.
fn resize_case(ws: &mut Workspace, playback: &mut Option<Playback>, id: &str, name: &str) -> Case {
    let mut case = Case::new(id, "workspace", name);
    let Some(divider) = ws.editor.read_with(&ws.vcx, |e, _| {
        let right = e.layout.docks.iter().position(|d| d.id == "right")?;
        e.dividers()
            .iter()
            .find(|d| d.dock == right && d.group.is_none())
            .map(|d| d.rect)
    }) else {
        return case.skip("no divider for the right dock");
    };
    let start = point(
        px(divider.x + divider.w / 2.),
        px(divider.y + divider.h / 2.),
    );
    let before = ws.rect("inspector").map(|r| r.w);
    ws.vcx.simulate_event(MouseDownEvent {
        position: start,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    let mut x = f32::from(start.x);
    for k in 0..iterations().max(16) {
        x += if (k / 8) % 2 == 0 { -6. } else { 6. };
        let at = point(px(x), start.y);
        let sample = step(
            ws,
            playback,
            |ws, probe| {
                probe.run(|| {
                    ws.vcx.simulate_event(MouseMoveEvent {
                        position: at,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::none(),
                    })
                })
            },
            false,
        );
        case.samples.push(sample);
    }
    let during = ws.rect("inspector").map(|r| r.w);
    ws.vcx.simulate_event(MouseUpEvent {
        position: point(px(x), start.y),
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 1,
    });
    ws.vcx.run_until_parked();
    case.note(
        "inspector_width",
        json!({"before": before, "during": during}),
    );
    case
}

/// The surfaces whose scrolling the owner hit, each wheel and trackpad.
const SCROLLS: [(&str, &str, &str, bool); 7] = [
    ("W1", "Properties (8 effects)", "inspector", false),
    ("W2", "Effect Controls", "layer-effects", false),
    ("W3", "Colour", "color", false),
    ("W4", "Effects browser", "fxbrowser", false),
    ("W5", "Project panel", "assets", false),
    ("W6", "Timeline, vertical", "timeline", false),
    ("W7", "Timeline, horizontal", "timeline", true),
];

/// `PM_UI_LATENCY_CASES=K1,K1p` runs only those workspace cases (profiling).
fn wanted(id: &str) -> bool {
    std::env::var("PM_UI_LATENCY_CASES")
        .map(|ids| ids.split(',').any(|i| i.trim() == id))
        .unwrap_or(true)
}

fn run_all(ws: &mut Workspace, playback: &mut Option<Playback>, out: &mut Vec<Case>) {
    let tag = if playback.is_some() { "p" } else { "" };
    let when = if playback.is_some() { ", playing" } else { "" };
    let mut push = |case: Case, ws: &mut Workspace, playback: &mut Option<Playback>| {
        let mut case = case;
        if let Some(p) = playback.as_mut() {
            p.note(&mut case);
        }
        case.note("publications", json!(std::mem::take(&mut ws.published)));
        out.push(case);
    };
    for (id, what, panel, horizontal) in SCROLLS {
        ws.show_tab(panel);
        for momentum in [false, true] {
            let (kind, suffix) = if momentum {
                ("Trackpad flick", "m")
            } else {
                ("Wheel", "w")
            };
            let id = format!("{id}{suffix}{tag}");
            if !wanted(&id) {
                continue;
            }
            ws.published.clear();
            let case = scroll_case(
                ws,
                playback,
                &id,
                &format!("{kind}: {what}{when}"),
                panel,
                horizontal,
                momentum,
            );
            push(case, ws, playback);
        }
    }
    for (id, what, panel) in [
        ("H1", "Properties", "inspector"),
        ("H2", "Effect Controls", "layer-effects"),
        ("H3", "Effects browser", "fxbrowser"),
        ("H4", "Project panel", "assets"),
        ("H5", "timeline", "timeline"),
    ] {
        let id = format!("{id}{tag}");
        if !wanted(&id) {
            continue;
        }
        ws.show_tab(panel);
        ws.published.clear();
        let case = hover_case(
            ws,
            playback,
            &id,
            &format!("Hover across {what} rows{when}"),
            panel,
        );
        push(case, ws, playback);
    }
    if wanted(&format!("K1{tag}")) {
        ws.published.clear();
        let case = select_case(
            ws,
            playback,
            &format!("K1{tag}"),
            &format!("Select a layer with every panel open{when}"),
        );
        push(case, ws, playback);
    }
    if wanted(&format!("K2{tag}")) {
        ws.published.clear();
        let case = tabs_case(
            ws,
            playback,
            &format!("K2{tag}"),
            &format!("Switch tabs (Project / Effect Controls){when}"),
        );
        push(case, ws, playback);
    }
    if wanted(&format!("K3{tag}")) {
        ws.published.clear();
        let case = toggle_case(
            ws,
            playback,
            &format!("K3{tag}"),
            &format!("Close / open a panel (Effects){when}"),
        );
        push(case, ws, playback);
    }
    if wanted(&format!("K4{tag}")) {
        ws.published.clear();
        let case = resize_case(
            ws,
            playback,
            &format!("K4{tag}"),
            &format!("Resize the right dock (per move){when}"),
        );
        push(case, ws, playback);
    }
}

pub(super) fn workspace_cases(cx: &mut TestAppContext, out: &mut Vec<Case>) {
    let mut ws = match workspace(cx) {
        Ok(ws) => ws,
        Err(why) => {
            out.push(Case::new("W0", "workspace", "Open the workspace").skip(why));
            return;
        }
    };
    run_all(&mut ws, &mut None, out);
    if std::env::var("PM_UI_LATENCY_CASES").is_ok_and(|ids| !ids.contains('p')) {
        return;
    }
    // A busy machine can hold the engine's first frames past the wait
    // (seen once in six runs): a second viewer before the cases count it a
    // failure, which the gate reports.
    let started = Playback::start(&ws)
        .or_else(|first| Playback::start(&ws).map_err(|again| format!("{first}; again: {again}")));
    let mut playback = match started {
        Ok(p) => Some(p),
        Err(why) => {
            out.push(Case::new("W0p", "workspace", "Start playback").skip(why));
            return;
        }
    };
    run_all(&mut ws, &mut playback, out);
}
