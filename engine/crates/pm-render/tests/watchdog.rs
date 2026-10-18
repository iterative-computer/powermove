//! The GPU watchdog's guards through `Renderer::render` with the
//! deterministic test hooks (`effects::watchdog::TimingSource` and the
//! clock; issue A2): the per-program budget records only, as today's
//! compositor (`legacy/gl/gpu-timing.ts`) does, and when a renderer opts
//! in to enforcing it, built-ins are never held to it; the hang guard's
//! budget is Chromium's (10 s) in the viewer too; the hang
//! guard's suspicion (a frame seen running past the budget holds frames
//! that draw its user programs) and attribution (only a program whose
//! timestamps show it ran away, or that the frame stopped inside, is turned
//! off; programs that finished, built-ins, and frames slowed by contention
//! turn nothing off; without timestamps two such frames are needed); the job
//! policy (no budget; a runaway stops the job with a named error until the
//! watchdog is reset); and the real completion path, where a frame whose
//! completion is seen late is not a runaway. The GPU timestamp path is
//! `effects.rs` `the_watchdog_judges_every_timed_frame_not_only_the_first`.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_protocol::definitions::{DefinitionKind, DefinitionOrigin, DefinitionSnapshot};
use pm_protocol::error::{ErrorCode, ErrorScope};
use pm_render::effects::watchdog::{FnTiming, Policy, Span, SpanTime, Subject, TimingSource};
use pm_render::{MapResolver, RenderError, RenderRequest, Renderer, builtin_definitions};
use serde_json::{Value, json};

/// Tests take turns on the GPU (one device per process in the engines).
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn layer(id: &str, ty: &str, d: Value, fx: Value) -> Value {
    json!({
        "id": id, "type": ty, "name": id, "from": 0, "dur": 10, "on": true, "lock": false,
        "threeD": false, "solo": false, "blend": "normal", "mblur": false, "parent": null,
        "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(0)),
            "position.y": ch(json!(0)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": fx, "transitionIn": null, "transitionOut": null, "masks": [], "d": d
    })
}

fn document(layers: Vec<Value>) -> pm_model::Document {
    let doc = json!({
        "id": "P", "name": "p", "w": 64, "h": 32, "fps": 30, "dur": 10, "bg": "#000000",
        "backgroundFill": { "type": "none", "angle": 0, "stops": [{ "id": "s1", "color": "#000000", "position": 0 }] },
        "compId": "P-root", "compName": "p", "markers": [], "work": [0, 10], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": layers, "assets": {}, "comps": {}
    });
    pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1)
}

fn shader_layer(code: &str) -> Value {
    layer(
        "sky",
        "shader",
        json!({ "code": code, "uniforms": {}, "w": 64, "h": 32 }),
        json!([]),
    )
}

/// A solid with `effects` (instance ids `f1`, `f2`, ...).
fn solid_with(effects: &[&str]) -> Value {
    let fx: Vec<Value> = effects
        .iter()
        .enumerate()
        .map(|(i, ty)| json!({ "id": format!("f{}", i + 1), "type": ty, "on": true, "p": {} }))
        .collect();
    layer(
        "box",
        "solid",
        json!({ "color": "#4C8DFF", "w": 40, "h": 20 }),
        Value::Array(fx),
    )
}

fn render(r: &mut Renderer, doc: &pm_model::Document) -> Result<Vec<f32>, RenderError> {
    let mut request = RenderRequest::sized(Size::new(64, 32), Precision::Float16);
    request.background = false;
    let frame = r.render(
        doc,
        "P-root",
        Ticks::ZERO,
        &request,
        &MapResolver::default(),
    )?;
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    Ok(r.read_rgba(&frame).unwrap())
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

/// Every program takes `span_ms`; every frame `frame_ms` of wall time.
fn timing(span_ms: Arc<AtomicU64>, frame_ms: Arc<AtomicU64>) -> Arc<dyn TimingSource> {
    Arc::new(FnTiming(
        move |_: &Span| Some(Duration::from_millis(span_ms.load(Ordering::Relaxed))),
        move |_: &[Span]| Duration::from_millis(frame_ms.load(Ordering::Relaxed)),
    ))
}

/// The built-ins plus `acme.invert`, a local (user) copy of Invert.
fn with_user_invert() -> pm_protocol::definitions::DefinitionSetSnapshot {
    let mut set = builtin_definitions();
    let mut user: DefinitionSnapshot = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap()
        .clone();
    user.id = "acme.invert".into();
    user.origin = DefinitionOrigin::Local;
    set.definitions.push(user);
    set
}

const SKY: &str = "void main(){ fragColor = vec4(uv, 0.5, 1.0); }";

#[test]
fn slow_programs_are_recorded_and_never_turned_off_in_the_viewer() {
    // Today's compositor times programs (`gpu-timing.ts`) and never turns
    // one off: a user effect, a shader layer and a built-in that each take
    // 3 s of GPU time a frame, finite and under Chromium's 10 s, draw on
    // every frame (amendment 22).
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    assert_eq!(r.effects().watchdog.budget(), None, "records only");
    assert_eq!(r.effects().watchdog.hang(), Duration::from_secs(10));
    let source = Scripted::new(
        vec![("acme.invert", ms(3_000)), ("invert", ms(3_000)), ("sky", ms(3_000))],
        9_500,
    );
    r.effects_mut().watchdog.set_timing_source(Some(source));
    let doc = document(vec![
        solid_with(&["invert", "acme.invert"]),
        shader_layer(SKY),
    ]);
    let first = render(&mut r, &doc).unwrap();
    for _ in 0..3 {
        assert_eq!(render(&mut r, &doc).unwrap(), first);
        r.wait_idle();
        assert!(r.watchdog_reports(true).is_empty());
        assert!(!r.watchdog_holding());
    }
    assert!(!disabled(&r, "acme.invert"));
    assert!(!disabled(&r, "invert"));
    assert_eq!(
        r.effects()
            .watchdog
            .measured_ms(DefinitionKind::Effect, "acme.invert"),
        Some(3_000.0),
        "recorded"
    );
    // The same frames without the user effect differ: it drew.
    let without = render(
        &mut r,
        &document(vec![solid_with(&["invert"]), shader_layer(SKY)]),
    )
    .unwrap();
    assert!(max_diff(&first, &without) > 0.1);
}

#[test]
fn an_enforced_budget_never_turns_a_built_in_off() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    r.effects_mut().watchdog.set_budget(Duration::from_millis(250));
    let source = Scripted::new(vec![("invert", ms(900)), ("acme.invert", ms(900))], 1_900);
    r.effects_mut().watchdog.set_timing_source(Some(source));
    render(&mut r, &document(vec![solid_with(&["invert", "acme.invert"])])).unwrap();
    r.wait_idle();
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(disabled(&r, "acme.invert"));
    assert!(!disabled(&r, "invert"), "a built-in is never held to it");
    r.effects_mut().watchdog.clear_budget();
    assert_eq!(r.effects().watchdog.budget(), None);
}

#[test]
fn an_enforced_budget_turns_a_shader_layer_off_until_its_code_changes() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    r.effects_mut().watchdog.set_budget(Duration::from_millis(250));
    let (span, frame) = (Arc::new(AtomicU64::new(300)), Arc::new(AtomicU64::new(16)));
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(timing(span.clone(), frame)));
    let empty = render(&mut r, &document(vec![])).unwrap();
    let doc = document(vec![shader_layer(SKY)]);
    let drawn = render(&mut r, &doc).unwrap();
    assert!(max_diff(&drawn, &empty) > 0.1, "the layer drew");
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::WatchdogTimeout);
    assert!(
        matches!(&reports[0].scope, ErrorScope::Layer { layer } if layer.as_str() == "sky"),
        "{:?}",
        reports[0].scope
    );
    assert!(
        reports[0].message.contains("layer sky"),
        "{}",
        reports[0].message
    );
    // From the next frame the layer draws nothing (as code that does not
    // compile), and is not reported again.
    span.store(1, Ordering::Relaxed);
    let off = render(&mut r, &doc).unwrap();
    assert!(max_diff(&off, &empty) <= 1.0 / 1024.0, "turned off");
    assert!(r.watchdog_reports(true).is_empty());
    let off = render(&mut r, &doc).unwrap();
    assert!(max_diff(&off, &empty) <= 1.0 / 1024.0, "still off");
    // Edited code is another program: it draws.
    let edited = document(vec![shader_layer(
        "void main(){ fragColor = vec4(uv, 0.5, 1.0); } // edited",
    )]);
    let again = render(&mut r, &edited).unwrap();
    assert!(
        max_diff(&again, &drawn) <= 1.0 / 1024.0,
        "edited code draws"
    );
    assert!(r.watchdog_reports(true).is_empty(), "1 ms is within budget");
    // `reset` (the user asked to try again) turns the old code on too.
    r.effects_mut().watchdog.reset();
    let back = render(&mut r, &doc).unwrap();
    assert!(max_diff(&back, &drawn) <= 1.0 / 1024.0);
}

/// Per-program times from `times` (by definition id, or "sky" for the
/// shader layer), every frame `frame_ms` of wall time, and still running
/// while `running` is set and the frame drew `acme.invert`.
struct Scripted {
    times: Mutex<Vec<(&'static str, SpanTime)>>,
    frame_ms: AtomicU64,
    running: AtomicBool,
}

impl Scripted {
    fn new(times: Vec<(&'static str, SpanTime)>, frame_ms: u64) -> Arc<Scripted> {
        Arc::new(Scripted {
            times: Mutex::new(times),
            frame_ms: AtomicU64::new(frame_ms),
            running: AtomicBool::new(false),
        })
    }

    fn set(&self, times: Vec<(&'static str, SpanTime)>) {
        *self.times.lock().unwrap() = times;
    }
}

fn name(span: &Span) -> String {
    match &span.subject {
        Subject::Definition { id, .. } => id.as_str().to_owned(),
        Subject::Code(_) => span.layer.as_str().to_owned(),
    }
}

impl TimingSource for Scripted {
    fn span(&self, span: &Span) -> SpanTime {
        let n = name(span);
        self.times
            .lock()
            .unwrap()
            .iter()
            .find(|(k, _)| *k == n)
            .map_or(SpanTime::Measured(Duration::from_millis(1)), |(_, t)| *t)
    }

    fn frame(&self, _: &[Span]) -> Duration {
        Duration::from_millis(self.frame_ms.load(Ordering::Relaxed))
    }

    fn running(&self, spans: &[Span]) -> bool {
        self.running.load(Ordering::Relaxed) && spans.iter().any(|s| name(s) == "acme.invert")
    }
}

fn ms(v: u64) -> SpanTime {
    SpanTime::Measured(Duration::from_millis(v))
}

fn disabled(r: &Renderer, id: &str) -> bool {
    r.effects().watchdog.is_disabled(DefinitionKind::Effect, id)
}

#[test]
fn a_runaway_program_is_turned_off_and_programs_that_finished_are_not() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    // The frame never finished in time: the system aborted it inside
    // acme.invert (its end never written). The built-in Invert and the
    // shader layer finished in a few milliseconds.
    let source = Scripted::new(
        vec![
            ("acme.invert", SpanTime::Unfinished),
            ("invert", ms(1)),
            ("sky", ms(3)),
        ],
        12_000,
    );
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(source.clone()));
    let doc = document(vec![
        solid_with(&["invert", "acme.invert"]),
        shader_layer(SKY),
    ]);
    let first = render(&mut r, &doc).unwrap();
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::WatchdogTimeout);
    assert!(
        matches!(&reports[0].scope,
            ErrorScope::Effect { definition, .. } if definition.as_str() == "acme.invert"),
        "{reports:?}"
    );
    assert!(
        reports[0].message.contains("did not finish"),
        "{}",
        reports[0].message
    );
    assert!(disabled(&r, "acme.invert"));
    assert!(!disabled(&r, "invert"), "built-ins cannot run away");
    // The next frame draws the built-in Invert and the shader layer and
    // skips the user's effect.
    source.frame_ms.store(16, Ordering::Relaxed);
    let next = render(&mut r, &doc).unwrap();
    let expected = render(
        &mut r,
        &document(vec![solid_with(&["invert"]), shader_layer(SKY)]),
    )
    .unwrap();
    assert!(max_diff(&next, &expected) <= 1.0 / 1024.0);
    assert!(max_diff(&next, &first) > 0.1);
    assert!(r.watchdog_reports(true).is_empty());
}

#[test]
fn a_program_measured_past_the_hang_budget_is_a_runaway() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    let source = Scripted::new(vec![("sky", ms(10_500))], 10_600);
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(source.clone()));
    render(&mut r, &document(vec![shader_layer(SKY)])).unwrap();
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(
        matches!(&reports[0].scope, ErrorScope::Layer { layer } if layer.as_str() == "sky"),
        "{reports:?}"
    );
    assert!(
        reports[0].message.contains("10.5 s"),
        "{}",
        reports[0].message
    );
}

#[test]
fn a_frame_slowed_by_contention_turns_nothing_off() {
    // The frame took 12 s of wall time (another process, an export on the
    // same GPU, a heavy frame of built-ins) but every program's timestamps
    // show it finished in milliseconds: nothing is turned off.
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    let source = Scripted::new(vec![("acme.invert", ms(2)), ("sky", ms(4))], 12_000);
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(source.clone()));
    let doc = document(vec![solid_with(&["acme.invert"]), shader_layer(SKY)]);
    let first = render(&mut r, &doc).unwrap();
    assert!(r.watchdog_reports(true).is_empty());
    for _ in 0..3 {
        assert_eq!(render(&mut r, &doc).unwrap(), first);
    }
    assert!(r.watchdog_reports(true).is_empty());
    assert!(!disabled(&r, "acme.invert"));
}

#[test]
fn without_timestamps_a_program_is_turned_off_after_two_frames_that_did_not_finish() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    let source = Scripted::new(vec![("acme.invert", SpanTime::Unknown)], 12_000);
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(source.clone()));
    let doc = document(vec![solid_with(&["acme.invert"])]);
    // Each frame is judged (with the source's times as they are now)
    // before the source changes.
    let step = |r: &mut Renderer| {
        let pixels = render(r, &doc).unwrap();
        r.wait_idle();
        (pixels, r.watchdog_reports(true))
    };
    let (first, reports) = step(&mut r);
    assert!(reports.is_empty(), "one unexplained hang");
    // A frame in between where it finished forgives it.
    source.frame_ms.store(16, Ordering::Relaxed);
    source.set(vec![("acme.invert", ms(1))]);
    assert_eq!(step(&mut r), (first.clone(), vec![]));
    source.frame_ms.store(12_000, Ordering::Relaxed);
    source.set(vec![("acme.invert", SpanTime::Unknown)]);
    assert_eq!(
        step(&mut r),
        (first.clone(), vec![]),
        "the count started over"
    );
    // Two unexplained hangs in a row: turned off.
    let (_, reports) = step(&mut r);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(
        reports[0].message.contains("2 frames"),
        "{}",
        reports[0].message
    );
    assert!(disabled(&r, "acme.invert"));
}

#[test]
fn a_frame_that_only_draws_built_ins_never_trips_the_hang_guard() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(
        r.set_definitions(&builtin_definitions())
            .unwrap()
            .is_empty()
    );
    let source = Scripted::new(
        vec![
            ("invert", SpanTime::Unfinished),
            ("blur", SpanTime::Unknown),
        ],
        12_000,
    );
    r.effects_mut().watchdog.set_timing_source(Some(source));
    let doc = document(vec![solid_with(&["invert", "blur"])]);
    let first = render(&mut r, &doc).unwrap();
    assert!(r.watchdog_reports(true).is_empty());
    assert_eq!(render(&mut r, &doc).unwrap(), first);
    assert!(r.watchdog_reports(true).is_empty());
}

#[test]
fn a_suspect_frame_holds_frames_drawing_its_user_programs_until_it_is_judged() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    let source = Scripted::new(vec![("acme.invert", ms(1))], 16);
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(source.clone()));
    let doc = document(vec![solid_with(&["acme.invert"])]);
    let other = document(vec![solid_with(&["invert"])]);
    let drawn = render(&mut r, &doc).unwrap();
    r.wait_idle();
    assert!(r.watchdog_reports(true).is_empty());
    // A frame drawing acme.invert is seen still running 11 s on.
    source.running.store(true, Ordering::Relaxed);
    source.frame_ms.store(11_000, Ordering::Relaxed);
    render(&mut r, &doc).unwrap();
    r.watchdog_watch();
    assert!(r.watchdog_holding());
    // Frames drawing it are held: nothing presented, no report, not off.
    for _ in 0..2 {
        match render(&mut r, &doc) {
            Err(RenderError::WatchdogPending) => {}
            other => panic!("expected a held frame, got {:?}", other.map(|_| ())),
        }
    }
    assert!(r.watchdog_reports(false).is_empty());
    assert!(!disabled(&r, "acme.invert"));
    // A frame without it draws.
    render(&mut r, &other).unwrap();
    // The frame ends, and its timestamps show acme.invert finished: the
    // hold ends and nothing is turned off.
    source.running.store(false, Ordering::Relaxed);
    r.watchdog_watch();
    assert!(!r.watchdog_holding());
    assert!(r.watchdog_reports(true).is_empty());
    assert_eq!(render(&mut r, &doc).unwrap(), drawn);
    r.wait_idle();
    // Seen running again; this time the frame stopped inside it.
    source.running.store(true, Ordering::Relaxed);
    render(&mut r, &doc).unwrap();
    r.watchdog_watch();
    assert!(matches!(
        render(&mut r, &doc),
        Err(RenderError::WatchdogPending)
    ));
    source.set(vec![("acme.invert", SpanTime::Unfinished)]);
    source.running.store(false, Ordering::Relaxed);
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(!r.watchdog_holding());
    // Turned off: the frame draws without it.
    let off = render(&mut r, &doc).unwrap();
    assert!(max_diff(&off, &drawn) > 0.1);
}

#[test]
fn a_job_renderer_has_no_budget_and_stops_after_a_runaway() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    r.effects_mut().watchdog.set_policy(Policy::Job);
    assert_eq!(r.effects().watchdog.hang(), Duration::from_secs(10));
    let source = Scripted::new(vec![("acme.invert", ms(900)), ("sky", ms(900))], 950);
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(source.clone()));
    let doc = document(vec![solid_with(&["acme.invert"]), shader_layer(SKY)]);
    // 900 ms per program is slow, not a runaway: an export draws it, as
    // today's exporter does.
    let first = render(&mut r, &doc).unwrap();
    assert!(r.watchdog_reports(true).is_empty());
    assert_eq!(render(&mut r, &doc).unwrap(), first);
    assert!(r.watchdog_reports(true).is_empty());
    assert_eq!(
        r.effects()
            .watchdog
            .measured_ms(DefinitionKind::Effect, "acme.invert"),
        Some(900.0)
    );
    // A job never holds: a frame seen running is waited for.
    source.running.store(true, Ordering::Relaxed);
    source.frame_ms.store(11_000, Ordering::Relaxed);
    render(&mut r, &doc).unwrap();
    r.watchdog_watch();
    assert!(!r.watchdog_holding());
    // That frame stopped inside acme.invert, and the shader ran 11 s: the
    // job stops with a named error, never a frame without them.
    source.set(vec![
        ("acme.invert", SpanTime::Unfinished),
        ("sky", ms(11_000)),
    ]);
    source.running.store(false, Ordering::Relaxed);
    r.wait_idle();
    assert_eq!(r.watchdog_reports(true).len(), 2);
    let stopped = r.watchdog_stopped().unwrap().to_owned();
    match render(&mut r, &doc) {
        Err(RenderError::Watchdog(message)) => {
            assert_eq!(message, stopped);
            assert!(message.contains("acme.invert"), "{message}");
            assert!(message.contains("layer sky"), "{message}");
            assert!(message.contains("10 s"), "{message}");
        }
        other => panic!(
            "expected the watchdog to stop the job, got {:?}",
            other.map(|_| ())
        ),
    }
    // The next job starts over.
    source.set(vec![]);
    source.frame_ms.store(16, Ordering::Relaxed);
    r.effects_mut().watchdog.reset();
    assert_eq!(render(&mut r, &doc).unwrap(), first);
}

/// A clock the test moves forward.
fn movable_clock() -> (Arc<AtomicU64>, pm_render::effects::watchdog::Clock) {
    let base = Instant::now();
    let offset = Arc::new(AtomicU64::new(0));
    let o = offset.clone();
    (
        offset,
        Arc::new(move || base + Duration::from_millis(o.load(Ordering::Relaxed))),
    )
}

#[test]
fn a_frame_whose_completion_is_seen_late_is_not_a_runaway() {
    // The real completion path (no timing source): the presenter submitted
    // a frame and then waited 12 s for the renderer's WebGL runner without
    // polling the device, so the completion callback fires 12 s after the
    // submission, past the 10 s hang budget. Before 2026-10-19 that read as
    // a runaway frame and turned every user program in it off.
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    let (offset, clock) = movable_clock();
    r.effects_mut().watchdog.set_clock(Some(clock));
    let doc = document(vec![solid_with(&["acme.invert"]), shader_layer(SKY)]);
    let first = render(&mut r, &doc).unwrap();
    for round in 1..=3u64 {
        offset.store(round * 12_000, Ordering::Relaxed);
        assert_eq!(render(&mut r, &doc).unwrap(), first);
        assert!(r.watchdog_reports(false).is_empty(), "round {round}");
        assert!(!r.watchdog_holding());
    }
    r.wait_idle();
    assert!(r.watchdog_reports(true).is_empty());
    assert!(!disabled(&r, "acme.invert"));
    assert_eq!(r.effects().watchdog.frames_in_flight(), 0);
}

/// A shader layer that takes tens of milliseconds at 1280 × 720.
fn heavy_sky(iterations: u32) -> Value {
    let code = format!(
        "void main(){{ float a = uv.x; for (int i = 0; i < {iterations}; i++) {{ a = sin(a * 1.0001 + 0.37); }} fragColor = vec4(a, uv.y, 0.5, 1.0); }}"
    );
    layer(
        "sky",
        "shader",
        json!({ "code": code, "uniforms": {}, "w": 1280, "h": 720 }),
        json!([]),
    )
}

#[test]
fn a_frame_seen_running_past_the_hang_budget_is_cleared_by_its_timestamps() {
    // The real path end to end: a frame is seen still running at a poll
    // the clock places 11 s after its submission (contention), the viewer
    // holds frames drawing its shader, and once it completes its GPU
    // timestamps show the shader took milliseconds: nothing is turned off.
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    if !r
        .device()
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        eprintln!("no TIMESTAMP_QUERY on this device");
        return;
    }
    let (offset, clock) = movable_clock();
    r.effects_mut().watchdog.set_clock(Some(clock));
    let mut request = RenderRequest::sized(Size::new(1280, 720), Precision::Float16);
    request.background = false;
    let mut seen = false;
    for (attempt, iterations) in [4_000u32, 16_000, 64_000].into_iter().enumerate() {
        let doc = document(vec![heavy_sky(iterations)]);
        // Warm: the program built, earlier frames judged.
        r.render(
            &doc,
            "P-root",
            Ticks::ZERO,
            &request,
            &MapResolver::default(),
        )
        .unwrap();
        r.wait_idle();
        assert!(r.watchdog_reports(true).is_empty());
        r.render(
            &doc,
            "P-root",
            Ticks::ZERO,
            &request,
            &MapResolver::default(),
        )
        .unwrap();
        offset.fetch_add(11_000, Ordering::Relaxed);
        r.watchdog_watch();
        if !r.watchdog_holding() {
            eprintln!("attempt {attempt}: the frame completed before the poll");
            r.wait_idle();
            assert!(r.watchdog_reports(true).is_empty());
            continue;
        }
        seen = true;
        assert!(matches!(
            r.render(
                &doc,
                "P-root",
                Ticks::ZERO,
                &request,
                &MapResolver::default()
            ),
            Err(RenderError::WatchdogPending)
        ));
        r.wait_idle();
        assert!(!r.watchdog_holding(), "judged once it completed");
        assert!(
            r.watchdog_reports(true).is_empty(),
            "cleared by its timestamps"
        );
        assert!(r.effects().watchdog.frames_in_flight() == 0);
        r.render(
            &doc,
            "P-root",
            Ticks::ZERO,
            &request,
            &MapResolver::default(),
        )
        .unwrap();
        break;
    }
    assert!(seen, "no frame was seen running");
}
