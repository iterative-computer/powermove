//! UI-latency catalogue: everyday small interactions measured headlessly,
//! through the real kernel, the real V8 panels, the v4 host and the native
//! timeline and viewer gizmos (no AppKit window, no OS input, no profile).
//!
//! Each case times, per iteration:
//! - `input_to_frame_ms`: from the input event to the first drawn window
//!   frame that shows its result (the kernel publication that carries it,
//!   the panel publication that shows it, and a full draw);
//! - `main_ms`: time spent on the test thread inside GPUI and the host
//!   (dispatch, layout, paint, publication apply), excluding waits on the
//!   kernel, V8 and engine threads. GPUI's test dispatcher runs "background"
//!   executor tasks on this thread too, so this over-counts work the app
//!   does off the main thread (documented in PLAN-UI.md).
//!
//! Frame pacing (owner directive 2026-11-21, "stuttery, not buttery smooth"):
//! averages hide stutter, so each case also records p99 and max, the
//! **hitch** count (iterations over 1.5 display intervals at 120 Hz
//! ProMotion, 12.5 ms), contiguous test-thread runs over **4 ms** (a run that
//! long leaves too little of an 8.33 ms frame), and the **cadence** a
//! continuous interaction would present on a synthetic 120 Hz clock
//! (skipped vsyncs, superseded results; see `cadence_120`).
//!
//! `PM_UI_LATENCY_OUT=/path.json` writes the catalogue; `PM_UI_LATENCY_BUDGET=1`
//! turns the budgets (p50 ≤ one 120 Hz frame, p95 ≤ 50 ms; and the pacing
//! gates p99 ≤ two frames, hitches ≤ `PM_UI_LATENCY_MAX_HITCHES` (0), runs
//! over 4 ms ≤ `PM_UI_LATENCY_MAX_MAIN_OVER_4MS` (0)) into assertions.
//! Without it the run only records (the `.20` baseline). A case that cannot
//! run headlessly is recorded as skipped with its reason, never a panic.
use crate::gizmo::{GizmoSurface, Host, Navigate, Tool, ViewFrame};
use crate::v4_view::{Natives, SurfaceView};
use crate::v4_view_tests::{PROJECT, Rig, publication, rig_project, tokens, window};
use gpui::{
    App, AppContext, Bounds, Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point, px,
};
use pm_engine_service::OverlayLine;
use pm_kernel::{Call, Caller, HistoryCall, Request, SelectionCall, WriteBody};
use pm_surface::events::EventDetail;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

/// One frame at 120 Hz.
const FRAME_MS: f64 = 1000. / 120.;
/// Never more than this at p95.
const P95_MS: f64 = 50.;
/// A hitch: longer than 1.5 display intervals at 120 Hz (12.5 ms).
const HITCH_MS: f64 = 1.5 * FRAME_MS;
/// Never more than this at p99: a result lands at worst one vsync late.
const P99_MS: f64 = 2. * FRAME_MS;
/// One contiguous main-thread run longer than this is flagged.
const MAIN_SLICE_MS: f64 = 4.;
/// How long one iteration may wait for its result before it counts as a miss.
const WAIT: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_micros(50);

fn iterations() -> usize {
    std::env::var("PM_UI_LATENCY_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &usize| *n >= 3)
        .unwrap_or(15)
}

/// Pacing gates: hitches and 4 ms runs allowed per case (default none).
fn pacing_allowance(var: &str) -> usize {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn layer_count() -> usize {
    std::env::var("PM_UI_LATENCY_LAYERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &usize| *n >= 4)
        .unwrap_or(60)
}

// ---------------------------------------------------------------- samples

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    input_to_frame_ms: f64,
    main_ms: f64,
    /// The result never showed within `WAIT`.
    missed: bool,
    /// The runs inside `main_ms` were timed one by one (`Probe`). A direct
    /// timing (`false`) is one contiguous run of `main_ms`.
    sliced: bool,
    /// The longest contiguous test-thread run.
    max_slice_ms: f64,
    /// Contiguous test-thread runs over `MAIN_SLICE_MS`.
    slices_over: u32,
    /// The whole-window draw (`draw`: next-frame callbacks, layout, paint).
    frame_ms: f64,
}

impl Sample {
    fn max_slice(&self) -> f64 {
        if self.sliced {
            self.max_slice_ms
        } else {
            self.main_ms
        }
    }
    fn slices_over(&self) -> u32 {
        if self.sliced {
            self.slices_over
        } else {
            u32::from(self.main_ms > MAIN_SLICE_MS)
        }
    }
}

/// Times one iteration: wall clock from the input, and the test thread's
/// own work (everything passed through `run`).
struct Probe {
    start: Instant,
    main: Duration,
    max_slice: Duration,
    slices_over: u32,
    frame: Duration,
}

impl Probe {
    fn start() -> Self {
        Self {
            start: Instant::now(),
            main: Duration::ZERO,
            max_slice: Duration::ZERO,
            slices_over: 0,
            frame: Duration::ZERO,
        }
    }
    fn run<R>(&mut self, f: impl FnOnce() -> R) -> R {
        let at = Instant::now();
        let out = f();
        let slice = at.elapsed();
        self.main += slice;
        self.max_slice = self.max_slice.max(slice);
        self.slices_over += u32::from(slice.as_secs_f64() * 1e3 > MAIN_SLICE_MS);
        out
    }
    fn elapsed_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1e3
    }
    fn sample(&self, missed: bool) -> Sample {
        Sample {
            input_to_frame_ms: self.elapsed_ms(),
            main_ms: self.main.as_secs_f64() * 1e3,
            missed,
            sliced: true,
            max_slice_ms: self.max_slice.as_secs_f64() * 1e3,
            slices_over: self.slices_over,
            frame_ms: self.frame.as_secs_f64() * 1e3,
        }
    }
}

/// Lets GPUI tasks and the kernel/V8 threads run until `done` holds.
fn pump_until(
    vcx: &VisualTestContext,
    probe: &mut Probe,
    mut done: impl FnMut(&VisualTestContext) -> bool,
) -> bool {
    loop {
        probe.run(|| vcx.run_until_parked());
        if probe.run(|| done(vcx)) {
            return true;
        }
        if probe.start.elapsed() > WAIT {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

/// A whole-window frame, as the next display-link tick draws it.
fn draw(vcx: &mut VisualTestContext, probe: &mut Probe) {
    let at = Instant::now();
    probe.run(|| {
        vcx.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.refresh();
        });
        vcx.run_until_parked();
    });
    probe.frame += at.elapsed();
}

#[derive(Default)]
struct Case {
    id: String,
    name: String,
    group: String,
    priority: bool,
    samples: Vec<Sample>,
    skipped: Option<String>,
    extra: serde_json::Map<String, Value>,
}

impl Case {
    fn new(id: &str, group: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            group: group.into(),
            name: name.into(),
            ..Default::default()
        }
    }
    fn priority(mut self) -> Self {
        self.priority = true;
        self
    }
    fn skip(mut self, why: impl Into<String>) -> Self {
        self.skipped = Some(why.into());
        self
    }
    fn note(&mut self, key: &str, value: Value) {
        self.extra.insert(key.into(), value);
    }
    fn add_extra_ms(&mut self, key: &str, values: &[f64]) {
        if values.is_empty() {
            return;
        }
        let (p50, p95, max) = stats(values);
        self.note(
            key,
            json!({"p50": p50, "p95": p95, "max": max, "n": values.len()}),
        );
    }
    fn measured(&self) -> Vec<f64> {
        self.samples.iter().map(|s| s.input_to_frame_ms).collect()
    }
    fn to_json(&self) -> Value {
        let frame: Vec<f64> = self.measured();
        let main: Vec<f64> = self.samples.iter().map(|s| s.main_ms).collect();
        let misses = self.samples.iter().filter(|s| s.missed).count();
        let summary = |v: &[f64]| {
            if v.is_empty() {
                Value::Null
            } else {
                let (p50, p95, max) = stats(v);
                json!({"p50": p50, "p95": p95, "p99": percentile(v, 0.99), "max": max})
            }
        };
        let slices: Vec<f64> = self.samples.iter().map(Sample::max_slice).collect();
        let frames: Vec<f64> = self
            .samples
            .iter()
            .filter(|s| s.sliced)
            .map(|s| s.frame_ms)
            .collect();
        json!({
            "id": self.id,
            "name": self.name,
            "group": self.group,
            "priority": self.priority,
            "iterations": self.samples.len(),
            "missed": misses,
            "skipped": self.skipped,
            "input_to_frame_ms": summary(&frame),
            "main_ms": summary(&main),
            "within_budget": self.within_budget(),
            "hitches": self.hitches(),
            "main_over_4ms": self.main_over(),
            "main_max_slice_ms": summary(&slices),
            "frame_ms": summary(&frames),
            "frame_over_4ms": frames.iter().filter(|ms| **ms > MAIN_SLICE_MS).count(),
            "cadence_120hz": cadence_120(&frame).to_json(),
            "within_pacing": self.within_pacing(),
            "extra": Value::Object(self.extra.clone()),
        })
    }
    /// Iterations whose result took longer than 1.5 frames at 120 Hz.
    fn hitches(&self) -> usize {
        self.samples
            .iter()
            .filter(|s| s.missed || s.input_to_frame_ms > HITCH_MS)
            .count()
    }
    /// Contiguous test-thread runs over 4 ms, across all iterations.
    fn main_over(&self) -> usize {
        self.samples.iter().map(|s| s.slices_over() as usize).sum()
    }
    /// The frame-pacing gates: p99, hitch count, 4 ms runs. `None` when the
    /// case has no samples (skipped).
    fn within_pacing(&self) -> Option<bool> {
        let v = self.measured();
        if v.is_empty() {
            return None;
        }
        Some(
            percentile(&v, 0.99) <= P99_MS
                && self.hitches() <= pacing_allowance("PM_UI_LATENCY_MAX_HITCHES")
                && self.main_over() <= pacing_allowance("PM_UI_LATENCY_MAX_MAIN_OVER_4MS"),
        )
    }
    /// `None` when the case has no samples (skipped).
    fn within_budget(&self) -> Option<bool> {
        let v = self.measured();
        if v.is_empty() {
            return None;
        }
        let (p50, p95, _) = stats(&v);
        Some(p50 <= FRAME_MS && p95 <= P95_MS && self.samples.iter().all(|s| !s.missed))
    }
}

fn stats(values: &[f64]) -> (f64, f64, f64) {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let at = |pct: f64| {
        let i = (((v.len() - 1) as f64) * pct).round() as usize;
        v[i.min(v.len() - 1)]
    };
    (at(0.5), at(0.95), v[v.len() - 1])
}

/// The value at `pct` (0..=1), as `stats` picks its percentiles.
fn percentile(values: &[f64], pct: f64) -> f64 {
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let i = (((v.len() - 1) as f64) * pct).round() as usize;
    v[i.min(v.len() - 1)]
}

/// What a continuous interaction (a drag, a scroll, hover) would present on
/// a synthetic 120 Hz display: input `k` arrives at vsync `k`, the main
/// thread handles the inputs in order, each for its measured input→frame
/// time, and each result shows at the first vsync at or after it is ready.
/// A model, not a capture: headless runs have no display link.
#[derive(Debug, Default, PartialEq)]
struct Cadence {
    /// Inputs (one per vsync).
    inputs: usize,
    /// Distinct vsyncs that showed a new result.
    presented: usize,
    /// Results that landed on a vsync another result also reached: drawn,
    /// never seen.
    superseded: usize,
    /// Vsyncs between two presented results that showed nothing new.
    skipped_vsyncs: usize,
    /// The longest wait between presented results, in vsyncs.
    max_gap_vsyncs: u64,
    /// Waits between presented results, ms.
    gaps_ms: Vec<f64>,
}

impl Cadence {
    fn to_json(&self) -> Value {
        let gaps = if self.gaps_ms.is_empty() {
            Value::Null
        } else {
            let (p50, p95, max) = stats(&self.gaps_ms);
            json!({"p50": p50, "p95": p95, "p99": percentile(&self.gaps_ms, 0.99), "max": max})
        };
        json!({
            "inputs": self.inputs,
            "presented": self.presented,
            "superseded": self.superseded,
            "skipped_vsyncs": self.skipped_vsyncs,
            "max_gap_vsyncs": self.max_gap_vsyncs,
            "gap_ms": gaps,
        })
    }
}

fn cadence_120(latencies_ms: &[f64]) -> Cadence {
    let mut out = Cadence {
        inputs: latencies_ms.len(),
        ..Default::default()
    };
    let mut ready = 0f64;
    let mut last: Option<u64> = None;
    for (k, ms) in latencies_ms.iter().enumerate() {
        let start = ready.max(k as f64 * FRAME_MS);
        ready = start + ms.max(0.);
        // A hair under a vsync boundary counts as that vsync.
        let vsync = ((ready / FRAME_MS) - 1e-9).ceil().max(k as f64 + 1.) as u64;
        match last {
            Some(prev) if vsync <= prev => out.superseded += 1,
            Some(prev) => {
                let gap = vsync - prev;
                out.presented += 1;
                out.skipped_vsyncs += (gap - 1) as usize;
                out.max_gap_vsyncs = out.max_gap_vsyncs.max(gap);
                out.gaps_ms.push(gap as f64 * FRAME_MS);
            }
            None => out.presented += 1,
        }
        last = Some(last.map_or(vsync, |prev| prev.max(vsync)));
    }
    out
}

// ---------------------------------------------------------------- project

/// `n` layers: `l0` a shape and `l1` an image side by side (picking, drags,
/// resizes), the rest small shapes along the bottom, out of their way. Every
/// layer has a real transform, so the viewer gizmo has poses for all.
fn project_json(n: usize) -> String {
    let channel = |key: &str, v: f64| format!(r#""{key}":{{"v":{v},"kf":[]}}"#);
    let layer = |i: usize| {
        let (kind, x, y, w) = match i {
            0 => ("shape", 200., 150., 100.),
            1 => ("image", 450., 150., 100.),
            _ => ("shape", 10. + ((i * 9) % 600) as f64, 320., 8.),
        };
        let channels = [
            ("opacity", 100.),
            ("position.x", x),
            ("position.y", y),
            ("scale.x", 100.),
            ("scale.y", 100.),
            ("anchor.x", 0.),
            ("anchor.y", 0.),
            ("rotation", 0.),
        ]
        .map(|(key, v)| channel(key, v))
        .join(",");
        format!(
            r##"{{"id":"l{i}","type":"{kind}","name":"Layer {i}","from":0,"dur":10,"on":true,"p":{{{channels}}},"d":{{"w":{w},"h":{w}}},"fx":[]}}"##
        )
    };
    let layers = (0..n).map(layer).collect::<Vec<_>>().join(",");
    format!(
        r##"{{"id":"integration","name":"UI Latency","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{{}},"comps":{{}},"layers":[{layers}]}}"##
    )
}

fn published(kernel: &pm_kernel::KernelHandle) -> std::sync::Arc<pm_kernel::Published> {
    kernel.published(PROJECT).expect("published project")
}

fn revision(kernel: &pm_kernel::KernelHandle) -> u64 {
    published(kernel).revision()
}

fn layer_info(kernel: &pm_kernel::KernelHandle, id: &str) -> Option<pm_kernel::model::LayerInfo> {
    let p = published(kernel);
    pm_kernel::model::list_layers(p.snapshot.root()?, None)
        .into_iter()
        .find(|l| l.id == id)
}

/// A kernel call that never blocks the test thread: the reply is polled
/// while GPUI keeps running (as the app awaits `Pending`).
fn call(
    kernel: &pm_kernel::KernelHandle,
    vcx: &VisualTestContext,
    probe: &mut Probe,
    call: impl Into<Call>,
) -> bool {
    let pending = probe.run(|| kernel.call(PROJECT, Request::new(Caller::native(), call)));
    let mut reply = None;
    pump_until(vcx, probe, |_| {
        reply = pending.try_take();
        reply.is_some()
    });
    matches!(reply, Some(Ok(ref ack)) if !matches!(ack.reply, pm_kernel::Reply::Throw(_)))
}

/// Untimed setup call (blocking, test thread only).
fn call_now(kernel: &pm_kernel::KernelHandle, call: impl Into<Call>) {
    let _ = kernel
        .call(PROJECT, Request::new(Caller::native(), call))
        .wait();
}

fn select_now(kernel: &pm_kernel::KernelHandle, ids: &[&str]) {
    call_now(
        kernel,
        SelectionCall::Layers {
            ids: ids.iter().map(|s| (*s).to_owned()).collect(),
            add: false,
        },
    );
}

// ------------------------------------------------------------ V8 panels

/// What `Controller::update_builtins` + `drain_builtins` do after a kernel
/// publication: every realm publishes, and the panel's view takes its new
/// tree. Returns how many publications reached the view.
fn republish(
    rig: &Rig,
    view: &Entity<SurfaceView>,
    vcx: &mut VisualTestContext,
    probe: &mut Probe,
) -> usize {
    let tickets = probe.run(|| rig.runtime.publish());
    for (_, ticket) in tickets {
        let Ok(ticket) = ticket else { continue };
        pump_until(vcx, probe, |_| ticket.try_take().is_some());
    }
    apply(rig, view, vcx, probe)
}

/// The view takes the panel's waiting publications (`drain_builtins`).
fn apply(
    rig: &Rig,
    view: &Entity<SurfaceView>,
    vcx: &mut VisualTestContext,
    probe: &mut Probe,
) -> usize {
    let panel = view.read_with(vcx, |v, _| v.publication.panel.clone());
    let mut n = 0;
    for p in rig.runtime.take_publications() {
        if p.panel == panel {
            probe.run(|| view.update(vcx, |v, cx| v.publish(p, cx)));
            n += 1;
        }
    }
    probe.run(|| vcx.run_until_parked());
    let _ = rig.runtime.take_errors();
    n
}

/// After an event sent to the panel's script: its handler ran, and the
/// publication it made (if any) is on screen.
fn await_panel(
    rig: &Rig,
    view: &Entity<SurfaceView>,
    vcx: &mut VisualTestContext,
    probe: &mut Probe,
    before: u64,
) -> bool {
    let idle = pump_until(vcx, probe, |vcx| view.read_with(vcx, |v, _| v.idle()));
    let mut shown = false;
    let deadline = Instant::now() + Duration::from_millis(400);
    while idle && Instant::now() < deadline {
        apply(rig, view, vcx, probe);
        if view.read_with(vcx, |v, _| v.publication.surface.revision()) != before {
            shown = true;
            break;
        }
        std::thread::sleep(POLL);
    }
    draw(vcx, probe);
    shown
}

/// The selector the host paints a surface node under (tests only).
fn selector(view: &Entity<SurfaceView>, vcx: &VisualTestContext, id: u64) -> &'static str {
    let name = view.read_with(vcx, |v, _| {
        v.prepared
            .node(id)
            .and_then(|n| n.semantic.as_ref().map(|s| s.to_string()))
            .unwrap_or_else(|| format!("n{id}"))
    });
    Box::leak(name.into_boxed_str())
}

fn painted(
    view: &Entity<SurfaceView>,
    vcx: &mut VisualTestContext,
    id: u64,
) -> Option<Bounds<Pixels>> {
    let name = selector(view, vcx, id);
    vcx.debug_bounds(name)
        .filter(|b| f32::from(b.size.width) > 1. && f32::from(b.size.height) > 1.)
}

/// Surface nodes, in tree order, that match.
fn nodes(
    view: &Entity<SurfaceView>,
    vcx: &VisualTestContext,
    pred: impl Fn(&pm_surface::tree::Node) -> bool,
) -> Vec<u64> {
    view.read_with(vcx, |v, _| {
        v.publication
            .surface
            .nodes()
            .values()
            .filter(|n| pred(n))
            .map(|n| n.id)
            .collect()
    })
}

fn role(n: &pm_surface::tree::Node) -> &str {
    n.props.get("role").and_then(Value::as_str).unwrap_or("")
}

struct PanelStage {
    rig: Rig,
    view: Entity<SurfaceView>,
    vcx: VisualTestContext,
}

fn panel_stage(cx: &mut TestAppContext, panel: &str, project: &str) -> PanelStage {
    let rig = rig_project(panel, project);
    let first = publication(&rig);
    let (view, mut vcx) = window(cx, &rig, first, Natives::default());
    let mut probe = Probe::start();
    republish(&rig, &view, &mut vcx, &mut probe);
    draw(&mut vcx, &mut probe);
    PanelStage { rig, view, vcx }
}

// -------------------------------------------------------- viewer gizmos

#[derive(Default)]
struct ViewerHost {
    overlays: Cell<usize>,
    overlay_at: Cell<Option<Instant>>,
    reports: RefCell<Vec<String>>,
}

struct Viewer(Rc<ViewerHost>);

impl Host for Viewer {
    fn time(&self, _: &App) -> f64 {
        0.5
    }
    fn playing(&self, _: &App) -> bool {
        false
    }
    fn frame(&self, bounds: Bounds<Pixels>, _: &App) -> Option<ViewFrame> {
        Some(ViewFrame {
            origin: bounds.origin + point(px(20.), px(20.)),
            scale: 1.,
            size: [640., 360.],
        })
    }
    fn tool(&self, _: &App) -> Tool {
        Tool::Select
    }
    fn overlay(&self, _: Vec<OverlayLine>, _: Option<crate::gizmo_layer::Placement>, _: &mut App) {
        self.0.overlays.set(self.0.overlays.get() + 1);
        self.0.overlay_at.set(Some(Instant::now()));
    }
    fn navigate(&self, _: Navigate, _: &mut App) {}
    fn layer_menu(&self, _: Option<String>, _: &mut gpui::Window, _: &mut App) {}
    fn report(&self, message: String, _: &mut App) {
        self.0.reports.borrow_mut().push(message);
    }
}

struct ViewerStage {
    rig: Rig,
    surface: Entity<GizmoSurface>,
    host: Rc<ViewerHost>,
    vcx: VisualTestContext,
    _view: Entity<SurfaceView>,
}

fn viewer_stage(cx: &mut TestAppContext, project: &str) -> ViewerStage {
    let rig = rig_project("viewer", project);
    select_now(&rig.kernel, &["l0"]);
    let first = publication(&rig);
    let natives = Natives::default();
    let host = Rc::new(ViewerHost::default());
    let kernel = rig.kernel.clone();
    let gizmo_host: Rc<dyn Host> = Rc::new(Viewer(host.clone()));
    let surface = cx.new(|_| GizmoSurface::new(kernel, PROJECT.into(), tokens(), gizmo_host));
    natives
        .views
        .borrow_mut()
        .insert("viewer.gizmos".into(), surface.clone().into());
    let (view, vcx) = window(cx, &rig, first, natives);
    let mut stage = ViewerStage {
        rig,
        surface,
        host,
        vcx,
        _view: view,
    };
    let mut probe = Probe::start();
    stage.refresh(&mut probe);
    stage
}

impl ViewerStage {
    /// What `Controller::refresh` does for the viewer on a publication:
    /// the gizmo reads the poses again and redraws its handles.
    fn refresh(&mut self, probe: &mut Probe) -> bool {
        probe.run(|| self.surface.update(&mut self.vcx, |s, cx| s.refresh(cx)));
        let surface = self.surface.clone();
        pump_until(&self.vcx, probe, move |vcx| {
            surface.read_with(vcx, |s, _| s.idle() && s.refresh_complete())
        })
    }
    fn idle(&self, probe: &mut Probe) -> bool {
        let surface = self.surface.clone();
        pump_until(&self.vcx, probe, move |vcx| {
            surface.read_with(vcx, |s, _| s.idle())
        })
    }
    fn origin(&mut self) -> Point<Pixels> {
        self.vcx
            .debug_bounds("viewer.gizmos")
            .map(|b| b.origin)
            .unwrap_or_default()
    }
    fn at(&mut self, comp: [f64; 2]) -> Point<Pixels> {
        self.origin() + point(px(20. + comp[0] as f32), px(20. + comp[1] as f32))
    }
    fn points(&self) -> Option<([f64; 2], [f64; 2], [f64; 2])> {
        self.surface.read_with(&self.vcx, |s, _| s.points())
    }
    fn undo_all(&mut self, steps: usize) {
        for _ in 0..steps {
            call_now(&self.rig.kernel, Call::History(HistoryCall::Undo));
        }
        let mut probe = Probe::start();
        self.refresh(&mut probe);
    }
}

/// One pointer drag of the selection's gizmo: every move is a sample, from
/// the move to the kernel's publication of it with the handles redrawn there.
fn viewer_drag(
    s: &mut ViewerStage,
    case: &mut Case,
    grab: impl Fn(([f64; 2], [f64; 2], [f64; 2])) -> [f64; 2],
    step: Point<Pixels>,
    modifiers: Modifiers,
) {
    let Some(points) = s.points() else {
        *case = std::mem::take(case).skip("the gizmo has no pose for the selection");
        return;
    };
    let from = s.at(grab(points));
    let history = published(&s.rig.kernel).history.clone();
    let rev_start = revision(&s.rig.kernel);
    s.vcx
        .simulate_mouse_down(from, MouseButton::Left, modifiers);
    let mut warm = Probe::start();
    s.idle(&mut warm);
    let (mut outline, mut kernel_ms, mut actor, mut publish_work) =
        (vec![], vec![], vec![], vec![]);
    for k in 1..=iterations() {
        let at = from + point(step.x * k as f32, step.y * k as f32);
        let before = revision(&s.rig.kernel);
        s.host.overlay_at.set(None);
        let mut probe = Probe::start();
        probe.run(|| s.vcx.simulate_mouse_move(at, MouseButton::Left, modifiers));
        if let Some(t) = s.host.overlay_at.get() {
            outline.push(t.duration_since(probe.start).as_secs_f64() * 1e3);
        }
        let kernel = s.rig.kernel.clone();
        let surface = s.surface.clone();
        let stepped = pump_until(&s.vcx, &mut probe, move |vcx| {
            surface.read_with(vcx, |g, _| g.idle()) && revision(&kernel) != before
        });
        kernel_ms.push(probe.elapsed_ms());
        let p = published(&s.rig.kernel);
        actor.push(p.actor_work.as_secs_f64() * 1e3);
        publish_work.push(p.publication_work.as_secs_f64() * 1e3);
        let refreshed = s.refresh(&mut probe);
        draw(&mut s.vcx, &mut probe);
        case.samples.push(probe.sample(!(stepped && refreshed)));
    }
    let end = from + point(step.x * iterations() as f32, step.y * iterations() as f32);
    s.vcx.simulate_mouse_up(end, MouseButton::Left, modifiers);
    let mut settle = Probe::start();
    s.idle(&mut settle);
    s.refresh(&mut settle);
    let after = published(&s.rig.kernel);
    case.add_extra_ms("outline_ms", &outline);
    case.add_extra_ms("kernel_publish_ms", &kernel_ms);
    case.add_extra_ms("kernel_actor_work_ms", &actor);
    case.add_extra_ms("kernel_publication_work_ms", &publish_work);
    case.note(
        "publications_per_drag",
        json!(after.revision().saturating_sub(rev_start)),
    );
    case.note(
        "undo_steps_per_drag",
        json!(after.history.applied as i64 - history.applied as i64),
    );
    case.note(
        "history_entries_delta",
        json!(after.history.entries as i64 - history.entries as i64),
    );
    case.note("host_reports", json!(*s.host.reports.borrow()));
    s.host.reports.borrow_mut().clear();
    let undo = (after.history.applied as i64 - history.applied as i64).max(0) as usize;
    s.undo_all(undo);
}

fn viewer_cases(cx: &mut TestAppContext, out: &mut Vec<Case>) {
    let project = project_json(layer_count());
    let mut s = viewer_stage(cx, &project);
    let px3 = point(px(3.), px(2.));

    // V1: drag a shape's body.
    let mut case = Case::new("V1", "viewer", "Drag a shape in the viewer (per move)").priority();
    select_now(&s.rig.kernel, &["l0"]);
    s.refresh(&mut Probe::start());
    viewer_drag(
        &mut s,
        &mut case,
        |(body, _, _)| [body[0] + 10., body[1] + 10.],
        px3,
        Modifiers::none(),
    );
    out.push(case);

    // V2: drag an image's body.
    let mut case = Case::new("V2", "viewer", "Drag an image in the viewer (per move)").priority();
    select_now(&s.rig.kernel, &["l1"]);
    s.refresh(&mut Probe::start());
    viewer_drag(
        &mut s,
        &mut case,
        |(body, _, _)| [body[0] + 10., body[1] + 10.],
        px3,
        Modifiers::none(),
    );
    out.push(case);

    // V3: resize a shape by a corner handle.
    let mut case = Case::new(
        "V3",
        "viewer",
        "Resize a shape by its corner handle (per move)",
    )
    .priority();
    select_now(&s.rig.kernel, &["l0"]);
    s.refresh(&mut Probe::start());
    viewer_drag(
        &mut s,
        &mut case,
        |(_, corner, _)| corner,
        px3,
        Modifiers::none(),
    );
    out.push(case);

    // V4: Shift-proportional resize of the image.
    let mut case = Case::new(
        "V4",
        "viewer",
        "Shift-resize an image proportionally (per move)",
    )
    .priority();
    select_now(&s.rig.kernel, &["l1"]);
    s.refresh(&mut Probe::start());
    let mut shift = Modifiers::none();
    shift.shift = true;
    viewer_drag(&mut s, &mut case, |(_, corner, _)| corner, px3, shift);
    out.push(case);

    // V5: rotate by the rotation handle.
    let mut case = Case::new("V5", "viewer", "Rotate by the rotation handle (per move)").priority();
    select_now(&s.rig.kernel, &["l0"]);
    s.refresh(&mut Probe::start());
    viewer_drag(
        &mut s,
        &mut case,
        |(_, _, knob)| knob,
        point(px(4.), px(0.)),
        Modifiers::none(),
    );
    out.push(case);

    // V6: arrow-key nudge, as `Controller::run_plan("nudgeSelection")` runs
    // it: plan on the publication, one transaction, publication, handles.
    let mut case = Case::new("V6", "viewer", "Nudge the selection with an arrow key").priority();
    select_now(&s.rig.kernel, &["l0"]);
    s.refresh(&mut Probe::start());
    let history = published(&s.rig.kernel).history.applied;
    let mut plan_ms = vec![];
    for _ in 0..iterations() {
        let mut probe = Probe::start();
        let p = published(&s.rig.kernel);
        let before = p.revision();
        let planned = Instant::now();
        let plan = crate::layer_commands::command_plan(
            "nudgeSelection",
            &[json!(1.0), json!(0.0)],
            &p,
            0.5,
        );
        plan_ms.push(planned.elapsed().as_secs_f64() * 1e3);
        let Some(call_) =
            plan.and_then(|plan| crate::layer_commands::transaction(&plan, Some(before)))
        else {
            case = case.skip("nudgeSelection planned nothing");
            break;
        };
        let ok = call(&s.rig.kernel, &s.vcx, &mut probe, call_);
        if !ok {
            case = case.skip("the kernel refused the nudge");
            break;
        }
        let kernel = s.rig.kernel.clone();
        let published_ = pump_until(&s.vcx, &mut probe, move |_| revision(&kernel) != before);
        let refreshed = s.refresh(&mut probe);
        draw(&mut s.vcx, &mut probe);
        case.samples
            .push(probe.sample(!(ok && published_ && refreshed)));
    }
    case.add_extra_ms("plan_ms", &plan_ms);
    case.note(
        "undo_steps_unsquashed",
        json!(published(&s.rig.kernel).history.applied as i64 - history as i64),
    );
    let undo = published(&s.rig.kernel)
        .history
        .applied
        .saturating_sub(history);
    s.undo_all(undo);
    out.push(case);

    // V7: click another layer in the viewer to select it.
    let mut case = Case::new(
        "V7",
        "viewer",
        "Click a layer in the viewer (hit-test + select)",
    );
    let targets = [("l1", [500., 200.]), ("l0", [250., 200.])];
    for k in 0..iterations() {
        let (id, comp) = targets[k % 2];
        let at = s.at(comp);
        let mut probe = Probe::start();
        probe.run(|| {
            s.vcx.simulate_event(MouseDownEvent {
                position: at,
                modifiers: Modifiers::none(),
                button: MouseButton::Left,
                click_count: 1,
                first_mouse: false,
            });
            s.vcx.simulate_event(MouseUpEvent {
                position: at,
                modifiers: Modifiers::none(),
                button: MouseButton::Left,
                click_count: 1,
            });
        });
        let kernel = s.rig.kernel.clone();
        let surface = s.surface.clone();
        let selected = pump_until(&s.vcx, &mut probe, move |vcx| {
            surface.read_with(vcx, |g, _| g.idle())
                && published(&kernel).selection.layers() == [id.to_owned()]
        });
        let refreshed = s.refresh(&mut probe);
        draw(&mut s.vcx, &mut probe);
        case.samples.push(probe.sample(!(selected && refreshed)));
    }
    out.push(case);

    // V8: hover a layer in the viewer (its hover box).
    let mut case = Case::new("V8", "viewer", "Hover a layer in the viewer");
    let spots = [([500., 200.], true), ([600., 40.], false)];
    for k in 0..iterations() {
        let (comp, over) = spots[k % 2];
        let at = s.at(comp);
        let mut probe = Probe::start();
        probe.run(|| s.vcx.simulate_mouse_move(at, None, Modifiers::none()));
        let surface = s.surface.clone();
        let shown = pump_until(&s.vcx, &mut probe, move |vcx| {
            surface.read_with(vcx, |g, _| g.hovered().is_some() == over)
        });
        draw(&mut s.vcx, &mut probe);
        case.samples.push(probe.sample(!shown));
    }
    out.push(case);
}

// --------------------------------------------- Properties (inspector)

fn properties_cases(cx: &mut TestAppContext, out: &mut Vec<Case>) {
    let project = project_json(layer_count());
    let mut s = panel_stage(cx, "inspector", &project);
    select_now(&s.rig.kernel, &["l0"]);
    let mut warm = Probe::start();
    republish(&s.rig, &s.view, &mut s.vcx, &mut warm);

    // P1: a transform value written per drag step (what a Properties scrub
    // or a viewer drag step costs Properties: kernel write, publication,
    // Properties republish, frame).
    let mut case = Case::new(
        "P1",
        "properties",
        "Drag a Position value in Properties (per step: write → Properties shows it)",
    )
    .priority();
    let history = published(&s.rig.kernel).history.applied;
    let mut actor = vec![];
    for k in 0..iterations() {
        let mut probe = Probe::start();
        let before = revision(&s.rig.kernel);
        let set =
            pm_kernel::PropertyCommand::Set(pm_kernel::commands::properties::SetProperty::new(
                "l0",
                "position.x",
                200. + k as f64 + 1.,
                0.,
            ));
        let ok = call(
            &s.rig.kernel,
            &s.vcx,
            &mut probe,
            Call::Transact {
                label: Some("Position".into()),
                expected_revision: None,
                body: WriteBody::Commands(vec![set.into()]),
            },
        );
        if !ok {
            case = case.skip("the kernel refused the Position write");
            break;
        }
        let kernel = s.rig.kernel.clone();
        let published_ = pump_until(&s.vcx, &mut probe, move |_| revision(&kernel) != before);
        actor.push(published(&s.rig.kernel).actor_work.as_secs_f64() * 1e3);
        let shown = republish(&s.rig, &s.view, &mut s.vcx, &mut probe) > 0;
        draw(&mut s.vcx, &mut probe);
        case.samples
            .push(probe.sample(!(ok && published_ && shown)));
    }
    case.add_extra_ms("kernel_actor_work_ms", &actor);
    case.note(
        "undo_steps",
        json!(published(&s.rig.kernel).history.applied as i64 - history as i64),
    );
    out.push(case);

    // P2: selection change → Properties shows the new layer.
    let mut case = Case::new("P2", "properties", "Selection change → Properties updated");
    for k in 0..iterations() {
        let id = if k % 2 == 0 { "l1" } else { "l0" };
        let mut probe = Probe::start();
        let ok = call(
            &s.rig.kernel,
            &s.vcx,
            &mut probe,
            SelectionCall::Layers {
                ids: vec![id.into()],
                add: false,
            },
        );
        let shown = republish(&s.rig, &s.view, &mut s.vcx, &mut probe) > 0;
        draw(&mut s.vcx, &mut probe);
        case.samples.push(probe.sample(!(ok && shown)));
    }
    out.push(case);

    // P3: hover across Properties' buttons.
    out.push(hover_controls(
        &mut s,
        "P3",
        "properties",
        "Hover across Properties controls",
    ));
}

/// Pointer moves across up to 16 painted controls: the host's hover wash.
fn hover_controls(s: &mut PanelStage, id: &str, group: &str, name: &str) -> Case {
    let mut case = Case::new(id, group, name);
    let ids = nodes(&s.view, &s.vcx, |n| {
        matches!(
            role(n),
            "button" | "tab" | "option" | "menuitem" | "row" | "listitem"
        )
    });
    let mut spots = vec![];
    for node in ids {
        if let Some(b) = painted(&s.view, &mut s.vcx, node) {
            spots.push(b.center());
        }
        if spots.len() >= 16 {
            break;
        }
    }
    if spots.len() < 2 {
        return case.skip("fewer than two painted controls");
    }
    for k in 0..iterations() {
        let at = spots[k % spots.len()];
        let mut probe = Probe::start();
        probe.run(|| s.vcx.simulate_mouse_move(at, None, Modifiers::none()));
        let view = s.view.clone();
        pump_until(&s.vcx, &mut probe, move |vcx| {
            view.read_with(vcx, |v, _| v.idle())
        });
        apply(&s.rig, &s.view, &mut s.vcx, &mut probe);
        draw(&mut s.vcx, &mut probe);
        case.samples.push(probe.sample(false));
    }
    case.note("controls", json!(spots.len()));
    case
}

/// Types into the panel's first text field, one character per sample, as
/// the native field reports it (`input` with the whole value).
fn type_into(s: &mut PanelStage, id: &str, group: &str, name: &str, words: &[&str]) -> Case {
    let mut case = Case::new(id, group, name);
    let Some(field) = nodes(&s.view, &s.vcx, |n| n.listeners.contains_key("input"))
        .into_iter()
        .next()
    else {
        return case.skip("no text field listens for input");
    };
    let mut values = vec![];
    for word in words {
        for end in 1..=word.len() {
            values.push(word[..end].to_owned());
        }
        values.push(String::new());
    }
    let mut published_ = 0;
    for k in 0..iterations() {
        let value = values[k % values.len()].clone();
        let before = s
            .view
            .read_with(&s.vcx, |v, _| v.publication.surface.revision());
        let mut probe = Probe::start();
        probe.run(|| {
            s.view.update(&mut s.vcx, |v, cx| {
                v.send(
                    field,
                    "input",
                    EventDetail {
                        value: Some(json!(value)),
                        ..Default::default()
                    },
                    cx,
                )
            })
        });
        let shown = await_panel(&s.rig, &s.view, &mut s.vcx, &mut probe, before);
        published_ += shown as usize;
        case.samples.push(probe.sample(false));
    }
    case.note("keystrokes_that_published", json!(published_));
    case
}

/// Wheel steps over the panel's middle, down then back up.
fn scroll(s: &mut PanelStage, id: &str, group: &str, name: &str) -> Case {
    let mut case = Case::new(id, group, name);
    let at = point(px(500.), px(320.));
    for k in 0..iterations() {
        let dy = if (k / 4) % 2 == 0 { -40. } else { 40. };
        let mut probe = Probe::start();
        probe.run(|| {
            s.vcx.simulate_event(ScrollWheelEvent {
                position: at,
                delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
                modifiers: Modifiers::none(),
                ..Default::default()
            })
        });
        let view = s.view.clone();
        pump_until(&s.vcx, &mut probe, move |vcx| {
            view.read_with(vcx, |v, _| v.idle())
        });
        apply(&s.rig, &s.view, &mut s.vcx, &mut probe);
        draw(&mut s.vcx, &mut probe);
        case.samples.push(probe.sample(false));
    }
    case
}

fn click_center(vcx: &mut VisualTestContext, probe: &mut Probe, at: Point<Pixels>) {
    probe.run(|| {
        vcx.simulate_event(MouseDownEvent {
            position: at,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        vcx.simulate_event(MouseUpEvent {
            position: at,
            modifiers: Modifiers::none(),
            button: MouseButton::Left,
            click_count: 1,
        });
    });
}

fn panel_cases(cx: &mut TestAppContext, out: &mut Vec<Case>) {
    let project = project_json(layer_count());
    // Effects.
    let mut s = panel_stage(cx, "fxbrowser", &project);
    out.push(type_into(
        &mut s,
        "E1",
        "effects",
        "Type in Effects search (filter update)",
        &["blur", "glow"],
    ));
    out.push(scroll(&mut s, "E2", "effects", "Scroll the Effects list"));
    out.push(hover_controls(
        &mut s,
        "E3",
        "effects",
        "Hover across Effects rows/buttons",
    ));
    // Project.
    let mut s = panel_stage(cx, "assets", &project);
    out.push(type_into(
        &mut s,
        "J1",
        "project",
        "Type in Project search",
        &["layer"],
    ));
    out.push(scroll(&mut s, "J2", "project", "Scroll the Project panel"));
    // Settings.
    let mut s = panel_stage(cx, "settings", &project);
    out.push(scroll(
        &mut s,
        "S1",
        "settings",
        "Scroll the Settings panel",
    ));
    let mut case = Case::new("S2", "settings", "Toggle a checkbox/switch in Settings");
    let toggles = nodes(&s.view, &s.vcx, |n| {
        matches!(role(n), "switch" | "checkbox")
    });
    let spot = toggles
        .into_iter()
        .find_map(|id| painted(&s.view, &mut s.vcx, id).map(|b| b.center()));
    match spot {
        None => case = case.skip("no painted switch or checkbox (Settings needs the app's facts)"),
        Some(at) => {
            for _ in 0..iterations() {
                let before = s
                    .view
                    .read_with(&s.vcx, |v, _| v.publication.surface.revision());
                let mut probe = Probe::start();
                click_center(&mut s.vcx, &mut probe, at);
                let shown = await_panel(&s.rig, &s.view, &mut s.vcx, &mut probe, before);
                case.samples.push(probe.sample(!shown));
            }
        }
    }
    out.push(case);
}

// ------------------------------------------------------------- timeline

fn timeline_cases(cx: &mut TestAppContext, out: &mut Vec<Case>) {
    use crate::timeline_tests::stage_project;
    let mut s = stage_project(cx, project_json(layer_count()));
    let row = |s: &mut crate::timeline_tests::Stage, id: &str| {
        s.vcx.debug_bounds(Box::leak(
            format!(r#"timeline:row:["layer","{id}"]"#).into_boxed_str(),
        ))
    };
    /// What the app does once the kernel published: the timeline syncs,
    /// every panel republishes, and the window draws.
    fn follow(s: &mut crate::timeline_tests::Stage, probe: &mut Probe) -> bool {
        probe.run(|| s.surface.update(&mut s.vcx, |t, cx| t.published(cx)));
        let surface = s.surface.clone();
        let idle = pump_until(&s.vcx, probe, move |vcx| {
            surface.read_with(vcx, |t, _| t.idle())
        });
        republish(&s.rig, &s.view, &mut s.vcx, probe);
        draw(&mut s.vcx, probe);
        idle
    }

    // T1: click a layer row.
    let mut case = Case::new("T1", "timeline", "Click a layer in the timeline");
    for k in 0..iterations() {
        let id = if k % 2 == 0 { "l1" } else { "l0" };
        let Some(b) = row(&mut s, id) else {
            case = case.skip("timeline rows were not painted");
            break;
        };
        let at = point(b.origin.x + px(12.), b.center().y);
        let mut probe = Probe::start();
        click_center(&mut s.vcx, &mut probe, at);
        let kernel = s.rig.kernel.clone();
        let selected = pump_until(&s.vcx, &mut probe, move |_| {
            published(&kernel).selection.layers() == [id.to_owned()]
        });
        let idle = follow(&mut s, &mut probe);
        case.samples.push(probe.sample(!(selected && idle)));
    }
    out.push(case);

    // T2: hover across layer rows (native highlight).
    let mut case = Case::new("T2", "timeline", "Hover across timeline layer rows");
    let spots: Vec<_> = (0..8)
        .filter_map(|i| row(&mut s, &format!("l{i}")).map(|b| b.center()))
        .collect();
    if spots.len() < 2 {
        case = case.skip("timeline rows were not painted");
    } else {
        for k in 0..iterations() {
            let at = spots[k % spots.len()];
            let mut probe = Probe::start();
            probe.run(|| s.vcx.simulate_mouse_move(at, None, Modifiers::none()));
            let surface = s.surface.clone();
            pump_until(&s.vcx, &mut probe, move |vcx| {
                surface.read_with(vcx, |t, _| t.idle())
            });
            draw(&mut s.vcx, &mut probe);
            case.samples.push(probe.sample(false));
        }
    }
    out.push(case);

    // T3/T4: the eye and the lock.
    let history = published(&s.rig.kernel).history.applied;
    for (case_id, control, layer, name) in [
        (
            "T3",
            "visibility",
            "l2",
            "Toggle a layer's eye (visibility)",
        ),
        ("T4", "lock", "l3", "Toggle a layer's lock"),
    ] {
        let mut case = Case::new(case_id, "timeline", name);
        for _ in 0..iterations() {
            if let Some(b) = row(&mut s, layer) {
                s.vcx
                    .simulate_mouse_move(b.center(), None, Modifiers::none());
                s.vcx.run_until_parked();
            }
            let selector =
                Box::leak(format!(r#"timeline:{control}:["layer","{layer}"]"#).into_boxed_str());
            let Some(target) = s.vcx.debug_bounds(selector) else {
                case = case.skip(format!("{control} control was not painted"));
                break;
            };
            let state = |kernel: &pm_kernel::KernelHandle| {
                layer_info(kernel, layer).map(|l| (l.visible, l.locked))
            };
            let before = state(&s.rig.kernel);
            let mut probe = Probe::start();
            click_center(&mut s.vcx, &mut probe, target.center());
            let kernel = s.rig.kernel.clone();
            let changed = pump_until(&s.vcx, &mut probe, move |_| state(&kernel) != before);
            let idle = follow(&mut s, &mut probe);
            case.samples.push(probe.sample(!(changed && idle)));
        }
        out.push(case);
    }

    // T5: expand / collapse a row's disclosure.
    let mut case = Case::new("T5", "timeline", "Expand / collapse a layer row");
    for _ in 0..iterations() {
        if let Some(b) = row(&mut s, "l0") {
            s.vcx
                .simulate_mouse_move(b.center(), None, Modifiers::none());
            s.vcx.run_until_parked();
        }
        let Some(target) = s.vcx.debug_bounds(r#"timeline:disclosure:["layer","l0"]"#) else {
            case = case.skip("disclosure control was not painted");
            break;
        };
        let mut probe = Probe::start();
        click_center(&mut s.vcx, &mut probe, target.center());
        let idle = follow(&mut s, &mut probe);
        case.samples.push(probe.sample(!idle));
    }
    out.push(case);

    // T6: wheel-scroll the rows.
    let mut case = Case::new("T6", "timeline", "Scroll the timeline");
    match s.vcx.debug_bounds("timeline:surface") {
        None => case = case.skip("timeline surface was not painted"),
        Some(b) => {
            for k in 0..iterations() {
                let dy = if (k / 4) % 2 == 0 { -40. } else { 40. };
                let mut probe = Probe::start();
                probe.run(|| {
                    s.vcx.simulate_event(ScrollWheelEvent {
                        position: b.center(),
                        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
                        modifiers: Modifiers::none(),
                        ..Default::default()
                    })
                });
                let surface = s.surface.clone();
                let idle = pump_until(&s.vcx, &mut probe, move |vcx| {
                    surface.read_with(vcx, |t, _| t.idle())
                });
                draw(&mut s.vcx, &mut probe);
                case.samples.push(probe.sample(!idle));
            }
        }
    }
    out.push(case);

    // T7: open a row's context menu (the menu call; AppKit draws it).
    let mut case = Case::new("T7", "timeline", "Open a layer's context menu");
    for _ in 0..iterations() {
        let Some(b) = row(&mut s, "l1") else {
            case = case.skip("timeline rows were not painted");
            break;
        };
        let at = point(b.origin.x + px(12.), b.center().y);
        let menus = s.recorder.menus.borrow().len();
        let mut probe = Probe::start();
        probe.run(|| {
            s.vcx.simulate_event(MouseDownEvent {
                position: at,
                modifiers: Modifiers::none(),
                button: MouseButton::Right,
                click_count: 1,
                first_mouse: false,
            });
            s.vcx.simulate_event(MouseUpEvent {
                position: at,
                modifiers: Modifiers::none(),
                button: MouseButton::Right,
                click_count: 1,
            });
        });
        let recorder = s.recorder.clone();
        let opened = pump_until(&s.vcx, &mut probe, move |_| {
            recorder.menus.borrow().len() > menus
        });
        draw(&mut s.vcx, &mut probe);
        case.samples.push(probe.sample(!opened));
    }
    out.push(case);

    // T8/T9: Undo and Redo of the eye/lock edits made above.
    let made = published(&s.rig.kernel)
        .history
        .applied
        .saturating_sub(history);
    for (case_id, name, history_call) in [
        ("T8", "Undo (⌘Z)", HistoryCall::Undo),
        ("T9", "Redo (⇧⌘Z)", HistoryCall::Redo),
    ] {
        let mut case = Case::new(case_id, "timeline", name);
        let n = made.min(iterations());
        if n == 0 {
            out.push(case.skip("no edits to undo (eye/lock did not write)"));
            continue;
        }
        for _ in 0..n {
            let before = revision(&s.rig.kernel);
            let mut probe = Probe::start();
            let ok = call(
                &s.rig.kernel,
                &s.vcx,
                &mut probe,
                Call::History(history_call.clone()),
            );
            if !ok {
                case = case.skip("the kernel refused the history call");
                break;
            }
            let kernel = s.rig.kernel.clone();
            let published_ = pump_until(&s.vcx, &mut probe, move |_| revision(&kernel) != before);
            let idle = follow(&mut s, &mut probe);
            case.samples.push(probe.sample(!(ok && published_ && idle)));
        }
        out.push(case);
    }
}

// --------------------------------------- shared per-publication costs

/// What `Controller::refresh` does with the workspace on every publication
/// (generated panels follow authored controls). Mirrors session.rs at each
/// commit of this branch, so the catalogue measures the code the app runs.
fn refresh_workspace_step(
    builds: &mut std::collections::HashMap<String, crate::generated_panel::panel_build::State>,
    cache: &mut crate::generated_panel::WorkspaceJson,
    previous: &pm_kernel::Published,
    next: &pm_kernel::Published,
) {
    cache.follow(builds, &previous.workspace, &next.workspace);
}

fn shared_cases(out: &mut Vec<Case>) {
    for layers in [layer_count(), 300] {
        let actor = pm_kernel::KernelActor::spawn(pm_kernel::KernelConfig::default());
        let kernel = actor.handle();
        let opened = kernel
            .open(
                PROJECT,
                pm_kernel::OpenSource::Untitled(
                    pm_model::json::parse(&project_json(layers)).unwrap(),
                ),
            )
            .wait();
        if !matches!(opened, Some(Ok(_))) {
            out.push(Case::new("X0", "shared", "Open the shared-cost project").skip("open failed"));
            return;
        }
        let tag = format!("{layers} layers");
        let mut rows = Case::new(
            &format!("X1-{layers}"),
            "shared",
            &format!("refresh: rows::list ({tag})"),
        );
        let mut disabled = Case::new(
            &format!("X2-{layers}"),
            "shared",
            &format!("refresh: layer_commands::disabled ({tag})"),
        );
        let mut workspace = Case::new(
            &format!("X3-{layers}"),
            "shared",
            &format!("refresh: workspace diff for generated panels ({tag})"),
        );
        let mut selection = Case::new(
            &format!("X4-{layers}"),
            "shared",
            &format!("kernel: selection change publication ({tag})"),
        );
        let mut builds = std::collections::HashMap::new();
        let mut cache = crate::generated_panel::WorkspaceJson::default();
        for k in 0..iterations() {
            let previous = kernel.published(PROJECT).unwrap();
            let id = format!("l{}", k % 2);
            let started = Instant::now();
            call_now(
                &kernel,
                SelectionCall::Layers {
                    ids: vec![id],
                    add: false,
                },
            );
            let p = kernel.published(PROJECT).unwrap();
            selection.samples.push(Sample {
                input_to_frame_ms: started.elapsed().as_secs_f64() * 1e3,
                main_ms: (p.actor_work + p.publication_work).as_secs_f64() * 1e3,
                missed: false,
                ..Default::default()
            });
            let t = Instant::now();
            let list = crate::rows::list(&p.snapshot);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            rows.samples.push(Sample {
                input_to_frame_ms: ms,
                main_ms: ms,
                missed: false,
                ..Default::default()
            });
            let t = Instant::now();
            let _ = crate::layer_commands::disabled(
                Some(&*p),
                &list,
            );
            let ms = t.elapsed().as_secs_f64() * 1e3;
            disabled.samples.push(Sample {
                input_to_frame_ms: ms,
                main_ms: ms,
                missed: false,
                ..Default::default()
            });
            let t = Instant::now();
            refresh_workspace_step(&mut builds, &mut cache, &previous, &p);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            workspace.samples.push(Sample {
                input_to_frame_ms: ms,
                main_ms: ms,
                missed: false,
                ..Default::default()
            });
        }
        workspace.note(
            "workspace_bytes",
            json!(
                pm_model::json::to_canonical_string(&kernel.published(PROJECT).unwrap().workspace)
                    .len()
            ),
        );
        out.extend([rows, disabled, workspace, selection]);
        drop(actor);
    }
}

// ------------------------------------------- gesture steps, many panels

/// The built-in panels the gesture case mounts (Properties first), and the
/// ones on screen: a default editor shows Properties, Project and the viewer
/// with Effect Controls, Effects, Color and Easing behind tabs.
const GESTURE_MOUNTED: [&str; 7] = [
    "inspector",
    "layer-effects",
    "assets",
    "fxbrowser",
    "color",
    "easing",
    "viewer",
];
const GESTURE_SHOWN: [&str; 3] = ["inspector", "assets", "viewer"];

/// G1: one step of a Properties scrub (a kernel gesture step) with seven
/// built-in panels running, as `Controller::update_builtins` publishes it:
/// the step is on screen once every realm asked has answered and Properties
/// took its tree. The baseline publishes every realm; ui-latency N2 only the
/// shown panels that show the edited layer, until the gesture ends.
fn gesture_cases(cx: &mut TestAppContext, out: &mut Vec<Case>) {
    let mut case = Case::new(
        "G1",
        "gesture",
        "Scrub step with 7 built-in panels running (write → Properties shows it)",
    )
    .priority();
    let project = project_json(layer_count());
    let mut rig = rig_project("inspector", &project);
    let Some(runtime) = std::sync::Arc::get_mut(&mut rig.runtime) else {
        out.push(case.skip("the panel runtime is shared"));
        return;
    };
    for panel in &GESTURE_MOUNTED[1..] {
        let Ok(ticket) = runtime.mount(panel) else {
            out.push(case.skip(format!("{panel} did not mount")));
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(180);
        let mounted = loop {
            if let Some(result) = ticket.try_take() {
                break result.is_ok();
            }
            if Instant::now() > deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        if !mounted {
            out.push(case.skip(format!("{panel} did not mount")));
            return;
        }
    }
    select_now(&rig.kernel, &["l0"]);
    let Some(first) = rig
        .runtime
        .take_publications()
        .into_iter()
        .find(|p| p.panel == "inspector")
    else {
        out.push(case.skip("Properties did not publish"));
        return;
    };
    let (view, mut vcx) = window(cx, &rig, first, crate::v4_view::Natives::default());
    let mut warm = Probe::start();
    republish(&rig, &view, &mut vcx, &mut warm);
    draw(&mut vcx, &mut warm);
    let begun = rig
        .kernel
        .call(
            PROJECT,
            Request::new(
                Caller::native(),
                pm_kernel::GestureCall::Begin {
                    label: "Position".into(),
                },
            ),
        )
        .wait();
    let Some(Ok(pm_kernel::Ack {
        reply: pm_kernel::Reply::Gesture(gesture),
        ..
    })) = begun
    else {
        out.push(case.skip("the kernel refused the gesture"));
        return;
    };
    let (mut realms, mut others) = (vec![], vec![]);
    for k in 0..iterations() {
        let mut probe = Probe::start();
        let before = revision(&rig.kernel);
        let set =
            pm_kernel::PropertyCommand::Set(pm_kernel::commands::properties::SetProperty::new(
                "l0",
                "position.x",
                300. + k as f64,
                0.,
            ));
        let ok = call(
            &rig.kernel,
            &vcx,
            &mut probe,
            pm_kernel::GestureCall::Step {
                gesture,
                body: WriteBody::Commands(vec![set.into()]),
                replace: true,
            },
        );
        let kernel = rig.kernel.clone();
        let published_ = pump_until(&vcx, &mut probe, move |_| revision(&kernel) != before);
        let tickets = probe.run(|| {
            // N2-SHIM-BEGIN
            let shown: std::collections::BTreeSet<String> =
                GESTURE_SHOWN.iter().map(|s| (*s).to_owned()).collect();
            match crate::gesture_publish::Deferral::default().scope(true, &shown) {
                crate::gesture_publish::Scope::Live(live) => {
                    rig.runtime.publish_live(|id| live.contains(id)).0
                }
                crate::gesture_publish::Scope::All => rig.runtime.publish(),
            }
            // N2-SHIM-END
        });
        realms.push(tickets.len() as f64);
        for (_, ticket) in tickets {
            let Ok(ticket) = ticket else { continue };
            pump_until(&vcx, &mut probe, |_| ticket.try_take().is_some());
        }
        // `drain_builtins`: every panel's new tree is taken on the main
        // thread; only Properties has a view here.
        let mut shown = false;
        let mut other = 0;
        for p in probe.run(|| rig.runtime.take_publications()) {
            if p.panel == "inspector" {
                probe.run(|| view.update(&mut vcx, |v, cx| v.publish(p, cx)));
                shown = true;
            } else {
                other += 1;
            }
        }
        others.push(other as f64);
        probe.run(|| vcx.run_until_parked());
        let _ = rig.runtime.take_errors();
        draw(&mut vcx, &mut probe);
        case.samples
            .push(probe.sample(!(ok && published_ && shown)));
    }
    let _ = rig
        .kernel
        .call(
            PROJECT,
            Request::new(Caller::native(), pm_kernel::GestureCall::Commit { gesture }),
        )
        .wait();
    case.add_extra_ms("realms_published_per_step", &realms);
    case.add_extra_ms("other_panel_trees_per_step", &others);
    out.push(case);
}

/// Interactions the headless platform cannot drive (AppKit windows, the
/// dock and menus live in pm-shell / the Controller, which needs a Metal
/// layer). Measured on the real Mac in the batched hold instead.
fn appkit_cases(out: &mut Vec<Case>) {
    for (id, name) in [
        ("A1", "Open & close a dropdown menu"),
        ("A2", "Open & close a popover"),
        ("A3", "Switch dock tab"),
        ("A4", "Switch panel / workspace pane"),
        ("A5", "Resize a dock divider"),
        ("A6", "Open Settings window / switch Settings pages"),
        ("A7", "Click Play / Pause"),
        ("A8", "Type in a layer rename field (AppKit field editor)"),
        ("A9", "Type in a numeric field (AppKit field editor)"),
    ] {
        out.push(Case::new(id, "appkit", name).skip(
            "needs the Controller and AppKit (window, dock, menus): real-Mac measurement only",
        ));
    }
}

// ----------------------------------------------------------------- runner

fn guarded(out: &mut Vec<Case>, group: &str, f: impl FnOnce(&mut Vec<Case>)) {
    let mut cases = vec![];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut cases)));
    out.extend(cases);
    if let Err(panic) = result {
        let why = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_else(|| "panic".into());
        out.push(Case::new(&format!("{group}!"), group, "group aborted").skip(why));
    }
}

fn report(cases: &[Case]) -> Value {
    json!({
        "version": 2,
        "frame_budget_ms": FRAME_MS,
        "p95_budget_ms": P95_MS,
        "pacing": {
            "display_hz": 120,
            "hitch_threshold_ms": HITCH_MS,
            "p99_budget_ms": P99_MS,
            "main_slice_budget_ms": MAIN_SLICE_MS,
            "max_hitches": pacing_allowance("PM_UI_LATENCY_MAX_HITCHES"),
            "max_main_over_4ms": pacing_allowance("PM_UI_LATENCY_MAX_MAIN_OVER_4MS"),
        },
        "iterations": iterations(),
        "layers": layer_count(),
        "commit": option_env!("PM_UI_LATENCY_COMMIT"),
        "cases": cases.iter().map(Case::to_json).collect::<Vec<_>>(),
    })
}

fn print(cases: &[Case]) {
    eprintln!(
        "\nUI latency (input→frame p50/p95/p99 ms | main p50/p95 ms | hitches >12.5 ms, runs >4 ms, skipped vsyncs)"
    );
    for c in cases {
        match (&c.skipped, c.samples.is_empty()) {
            (Some(why), _) => eprintln!("{:<8} {:<58} skipped: {why}", c.id, c.name),
            (None, true) => eprintln!("{:<8} {:<58} no samples", c.id, c.name),
            (None, false) => {
                let (f50, f95, _) = stats(&c.measured());
                let f99 = percentile(&c.measured(), 0.99);
                let main: Vec<f64> = c.samples.iter().map(|s| s.main_ms).collect();
                let (m50, m95, _) = stats(&main);
                let cadence = cadence_120(&c.measured());
                eprintln!(
                    "{:<8} {:<58} {f50:8.2} {f95:8.2} {f99:8.2} | {m50:8.2} {m95:8.2} | {:3} {:3} {:3}{}{}",
                    c.id,
                    c.name,
                    c.hitches(),
                    c.main_over(),
                    cadence.skipped_vsyncs,
                    if c.within_budget() == Some(true) {
                        ""
                    } else {
                        "  over"
                    },
                    if c.within_pacing() == Some(true) {
                        ""
                    } else {
                        "  stutter"
                    }
                );
            }
        }
    }
}

#[gpui::test]
fn ui_latency_catalogue(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let mut cases = vec![];
    // Priority first: the tester's "moving and resizing shapes lags".
    guarded(&mut cases, "viewer", |out| viewer_cases(cx, out));
    guarded(&mut cases, "properties", |out| properties_cases(cx, out));
    guarded(&mut cases, "gesture", |out| gesture_cases(cx, out));
    guarded(&mut cases, "timeline", |out| timeline_cases(cx, out));
    guarded(&mut cases, "panels", |out| panel_cases(cx, out));
    guarded(&mut cases, "shared", shared_cases);
    appkit_cases(&mut cases);
    print(&cases);
    if let Some(path) = std::env::var_os("PM_UI_LATENCY_OUT") {
        let text = serde_json::to_string_pretty(&report(&cases)).unwrap();
        std::fs::write(&path, text).expect("write PM_UI_LATENCY_OUT");
    }
    if std::env::var("PM_UI_LATENCY_BUDGET").as_deref() == Ok("1") {
        let over: Vec<String> = cases
            .iter()
            .filter(|c| c.within_budget() == Some(false))
            .map(|c| {
                let (p50, p95, _) = stats(&c.measured());
                format!("{} {} (p50 {p50:.2} ms, p95 {p95:.2} ms)", c.id, c.name)
            })
            .collect();
        let stutter: Vec<String> = cases
            .iter()
            .filter(|c| c.within_pacing() == Some(false))
            .map(|c| {
                format!(
                    "{} {} (p99 {:.2} ms, {} hitches > {HITCH_MS:.1} ms, {} runs > {MAIN_SLICE_MS} ms)",
                    c.id,
                    c.name,
                    percentile(&c.measured(), 0.99),
                    c.hitches(),
                    c.main_over()
                )
            })
            .collect();
        assert!(
            over.is_empty() && stutter.is_empty(),
            "over the 8.33 ms p50 / 50 ms p95 budget:\n{}\nover the pacing gates (p99 ≤ {P99_MS:.2} ms, hitches, 4 ms runs):\n{}",
            over.join("\n"),
            stutter.join("\n")
        );
    }
}

/// The catalogue's own arithmetic (no GPUI): percentiles and budgets.
#[test]
fn ui_latency_stats_and_budget() {
    let (p50, p95, max) = stats(&[5., 1., 3., 2., 4.]);
    assert_eq!((p50, max), (3., 5.));
    assert_eq!(p95, 5.);
    let mut case = Case::new("Z", "test", "budget");
    assert_eq!(case.within_budget(), None);
    case.samples = (0..20)
        .map(|i| Sample {
            input_to_frame_ms: if i == 19 { 40. } else { 4. },
            main_ms: 1.,
            missed: false,
            ..Default::default()
        })
        .collect();
    assert_eq!(case.within_budget(), Some(true));
    // The one 40 ms iteration passes p50/p95 but is a hitch and the p99.
    assert_eq!(case.hitches(), 1);
    assert_eq!(percentile(&case.measured(), 0.99), 40.);
    assert_eq!(case.within_pacing(), Some(false));
    case.samples[19].input_to_frame_ms = 4.;
    assert_eq!(case.within_pacing(), Some(true));
    // A direct timing is one run; a probed one counts its own runs.
    case.samples[3].main_ms = 6.;
    assert_eq!(case.main_over(), 1);
    assert_eq!(case.within_pacing(), Some(false));
    case.samples[3] = Sample {
        input_to_frame_ms: 4.,
        main_ms: 6.,
        sliced: true,
        max_slice_ms: 3.,
        ..Default::default()
    };
    assert_eq!(case.main_over(), 0);
    assert_eq!(case.within_pacing(), Some(true));
    case.samples[0].missed = true;
    assert_eq!(case.within_budget(), Some(false));
    assert_eq!(case.hitches(), 1);
    // Cadence on a synthetic 120 Hz clock: every result inside its frame
    // shows on the next vsync, nothing skipped.
    let smooth = cadence_120(&[4.; 6]);
    assert_eq!(
        (smooth.presented, smooth.superseded, smooth.skipped_vsyncs),
        (6, 0, 0)
    );
    assert_eq!(smooth.max_gap_vsyncs, 1);
    // One 20 ms result: two vsyncs show nothing new, and the queued result
    // behind it lands on the same vsync (drawn, never seen).
    let hitch = cadence_120(&[4., 20., 4., 4.]);
    assert_eq!(hitch.presented, 3);
    assert_eq!(hitch.superseded, 1);
    assert_eq!(hitch.skipped_vsyncs, 2);
    assert_eq!(hitch.max_gap_vsyncs, 3);
    let project: Value = serde_json::from_str(&project_json(6)).unwrap();
    assert_eq!(project["layers"].as_array().unwrap().len(), 6);
    assert_eq!(project["layers"][1]["type"], "image");
}
