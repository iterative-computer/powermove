//! The GPU watchdog's guards through `Renderer::render` with the
//! deterministic test hook (`effects::watchdog::TimingSource`; issue A2):
//! the per-program budget for shader layers, the hang guard for runaway
//! programs (user programs off at once, built-ins kept), and the job
//! policy (no budget; a runaway stops the job with a named error until the
//! watchdog is reset). The GPU timestamp path is `effects.rs`
//! `the_watchdog_judges_every_timed_frame_not_only_the_first`.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_protocol::definitions::{DefinitionKind, DefinitionOrigin, DefinitionSnapshot};
use pm_protocol::error::{ErrorCode, ErrorScope};
use pm_render::effects::watchdog::{FnTiming, Policy, Span, TimingSource};
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
fn a_shader_layer_over_budget_is_turned_off_until_its_code_changes() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
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

#[test]
fn a_runaway_frame_turns_off_its_user_programs_and_keeps_built_ins() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    // Each program within the budget, but the frame never finished in
    // time: a runaway loop the GPU ran until the system aborted it.
    let (span, frame) = (Arc::new(AtomicU64::new(1)), Arc::new(AtomicU64::new(5_000)));
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(timing(span, frame.clone())));
    let doc = document(vec![
        solid_with(&["invert", "acme.invert"]),
        shader_layer(SKY),
    ]);
    let first = render(&mut r, &doc).unwrap();
    let mut reports = r.watchdog_reports(true);
    reports.sort_by_key(|e| format!("{:?}", e.scope));
    assert_eq!(reports.len(), 2, "{reports:?}");
    assert!(reports.iter().all(|e| e.code == ErrorCode::WatchdogTimeout));
    assert!(
        reports.iter().any(|e| matches!(&e.scope,
            ErrorScope::Effect { definition, .. } if definition.as_str() == "acme.invert")),
        "{reports:?}"
    );
    assert!(
        reports
            .iter()
            .any(|e| matches!(&e.scope, ErrorScope::Layer { layer } if layer.as_str() == "sky")),
        "{reports:?}"
    );
    let watchdog = &r.effects().watchdog;
    assert!(watchdog.is_disabled(DefinitionKind::Effect, "acme.invert"));
    assert!(
        !watchdog.is_disabled(DefinitionKind::Effect, "invert"),
        "built-ins cannot run away"
    );
    // The next frame draws the built-in invert, skips the user's, and
    // leaves the shader layer out.
    frame.store(16, Ordering::Relaxed);
    let next = render(&mut r, &doc).unwrap();
    let expected = render(&mut r, &document(vec![solid_with(&["invert"])])).unwrap();
    assert!(max_diff(&next, &expected) <= 1.0 / 1024.0);
    assert!(max_diff(&next, &first) > 0.1);
    assert!(r.watchdog_reports(true).is_empty());
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
    let (span, frame) = (Arc::new(AtomicU64::new(1)), Arc::new(AtomicU64::new(5_000)));
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(timing(span, frame)));
    let doc = document(vec![solid_with(&["invert", "blur"])]);
    let first = render(&mut r, &doc).unwrap();
    assert!(r.watchdog_reports(true).is_empty());
    assert_eq!(render(&mut r, &doc).unwrap(), first);
}

#[test]
fn a_job_renderer_has_no_budget_and_stops_after_a_runaway() {
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    r.effects_mut().watchdog.set_policy(Policy::Job);
    assert_eq!(r.effects().watchdog.hang(), Duration::from_secs(10));
    let (span, frame) = (Arc::new(AtomicU64::new(900)), Arc::new(AtomicU64::new(950)));
    r.effects_mut()
        .watchdog
        .set_timing_source(Some(timing(span.clone(), frame.clone())));
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
    // A frame past the job hang budget: the job stops with a named error,
    // never a frame without the program.
    frame.store(11_000, Ordering::Relaxed);
    render(&mut r, &doc).unwrap();
    assert_eq!(r.watchdog_reports(true).len(), 2);
    match render(&mut r, &doc) {
        Err(RenderError::Watchdog(message)) => {
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
    frame.store(16, Ordering::Relaxed);
    r.effects_mut().watchdog.reset();
    assert_eq!(render(&mut r, &doc).unwrap(), first);
}

#[test]
fn a_frame_still_running_past_the_hang_budget_is_judged_before_it_finishes() {
    // Without a test source: the wall clock. A 1 ns hang budget makes any
    // frame not yet complete at the next collect a runaway; it is judged
    // from the submission time, not when (if ever) it completes.
    let _gpu = gpu();
    let mut r = Renderer::headless().unwrap();
    assert!(r.set_definitions(&with_user_invert()).unwrap().is_empty());
    r.effects_mut().watchdog.set_hang(Duration::from_nanos(1));
    r.effects_mut().watchdog.set_budget(Duration::from_secs(60));
    let doc = document(vec![solid_with(&["acme.invert"])]);
    render(&mut r, &doc).unwrap();
    // Whether the frame completed or not, its wall time exceeds 1 ns.
    let reports = r.watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(
        r.effects()
            .watchdog
            .is_disabled(DefinitionKind::Effect, "acme.invert")
    );
    assert_eq!(r.effects().watchdog.frames_in_flight(), 0);
}
