//! The presenter's definition sets (contract 10) across pushes and views:
//! one renderer serves every view on a device, and each view keeps its own
//! programs and watchdog state. A push that does not change a definition's
//! source (any unrelated registration, dispose, or hot reload) keeps the
//! definition's program and its watchdog record, so an effect the watchdog
//! turned off stays off; another view drawing in between does not reset it.
//! A definition registered again (a delta's upsert) starts over.
//!
//! The budget records only in production (today's compositor never turns a
//! program off, amendment 22) and never applies to built-ins: this test
//! enforces it on view A for a user override of `blur` by id.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use pm_node::present::compose::Composer;
use pm_node::present::document::MediaMap;
use pm_protocol::color::Precision;
use pm_protocol::common::{Seq, Size, Ticks};
use pm_protocol::definitions::{DefinitionKind, DefinitionSetSnapshot, DefinitionSnapshot};
use pm_protocol::error::ErrorCode;
use pm_render::effects::watchdog::{FnTiming, Span, Subject, TimingSource};
use pm_render::{RenderRequest, Renderer};
use serde_json::{Value, json};

fn ch(v: Value) -> Value {
    json!({ "v": v, "kf": [], "expr": null })
}

fn document(fx: Value) -> pm_model::Document {
    let layer = json!({
        "id": "box", "type": "solid", "name": "box", "from": 0, "dur": 2, "on": true, "lock": false,
        "shy": false, "collapsed": true, "threeD": false, "solo": false, "color": "#4C8DFF",
        "blend": "normal", "mblur": false, "parent": null, "group": null, "matteSource": null,
        "p": {
            "anchor.x": ch(json!(0)), "anchor.y": ch(json!(0)), "position.x": ch(json!(140)),
            "position.y": ch(json!(80)), "scale.x": ch(json!(100)), "scale.y": ch(json!(100)),
            "rotation": ch(json!(0)), "opacity": ch(json!(100)), "skew": ch(json!(0))
        },
        "fx": fx, "transitionIn": null, "transitionOut": null, "masks": [],
        "d": { "color": "#E8E2CF", "w": 40, "h": 20 }, "locked_intent": {}
    });
    let project = json!({
        "id": "Pdefs", "name": "defs", "w": 320, "h": 180, "fps": 30, "dur": 2, "bg": "#000000",
        "backgroundFill": { "type": "solid", "angle": 0, "stops": [{ "id": "s1", "color": "#202020", "position": 0 }] },
        "compId": "Pdefs-root", "compName": "defs", "markers": [], "work": [0, 2], "params": {},
        "revision": 0, "edits": [], "created": 0, "shutter": 0.5, "layers": [layer], "assets": {}, "comps": {}
    });
    let value = pm_model::json::parse(&project.to_string()).unwrap();
    pm_model::Document::new(Some(value), 1)
}

fn blurred() -> pm_model::Document {
    document(json!([{ "id": "fx1", "type": "blur", "on": true, "p": { "amount": ch(json!(8)) } }]))
}

fn render(composer: &mut Composer, doc: &pm_model::Document) -> Vec<f32> {
    let request = RenderRequest::at_scale(Size::new(320, 180), 1.0, Precision::Float16);
    let r = composer.renderer();
    let frame = r
        .render(
            doc,
            "Pdefs-root",
            Ticks::ZERO,
            &request,
            &MediaMap::default(),
        )
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    r.read_rgba(&frame).unwrap()
}

fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

/// The built-ins plus one more definition (an unrelated registration).
fn with_extra(set: &DefinitionSetSnapshot, generation: u64) -> DefinitionSetSnapshot {
    let mut extra: DefinitionSnapshot = set
        .definitions
        .iter()
        .find(|d| d.id.as_str() == "invert")
        .unwrap()
        .clone();
    extra.id = "acme.invert".into();
    let mut next = set.clone();
    next.generation = Seq::new(generation).unwrap();
    next.definitions.push(extra);
    next
}

/// The watchdog's test hook (issue A2): the blur takes `blur_ms` of GPU
/// time per frame; frames take 16 ms. Every frame is judged at its
/// completion, on any device and under any load, so this test does not
/// depend on Metal writing every timestamp sample.
fn blur_takes(ms: Arc<AtomicU64>) -> Arc<dyn TimingSource> {
    Arc::new(FnTiming(
        move |span: &Span| {
            Some(match &span.subject {
                Subject::Definition { id, .. } if id.as_str() == "blur" => {
                    Duration::from_millis(ms.load(Ordering::Relaxed))
                }
                _ => Duration::from_millis(1),
            })
        },
        |_: &[Span]| Duration::from_millis(16),
    ))
}

#[test]
fn a_disabled_effect_stays_off_across_unrelated_pushes_and_other_views() {
    let headless = Renderer::headless().unwrap();
    let mut composer = Composer::new(headless.device(), headless.queue());
    let blur_ms = Arc::new(AtomicU64::new(300));
    let builtins = Arc::new({
        let mut set = pm_render::builtin_definitions();
        for d in set.definitions.iter_mut().filter(|d| d.id.as_str() == "blur") {
            d.origin = pm_protocol::definitions::DefinitionOrigin::Local;
        }
        set
    });
    let doc = blurred();
    let plain = document(json!([]));
    // View A draws the blur; an enforced 250 ms budget makes it overrun.
    let a = 1u64;
    let b = 2u64;
    composer.use_definitions(a, Some(&(builtins.clone(), 1)), &[]);
    let base = render(&mut composer, &plain);
    composer
        .renderer()
        .effects_mut()
        .watchdog
        .set_timing_source(Some(blur_takes(blur_ms.clone())));
    composer
        .renderer()
        .effects_mut()
        .watchdog
        .set_budget(Duration::from_millis(250));
    // Judged at the first frame's completion: 300 ms against 250.
    let first = render(&mut composer, &doc);
    let reports = composer.renderer().watchdog_reports(true);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(reports[0].code, ErrorCode::WatchdogTimeout);
    assert!(max_diff(&first, &base) > 0.05, "the blur drew");
    let off = render(&mut composer, &doc);
    assert!(max_diff(&off, &base) <= 1.0 / 1024.0, "turned off");
    assert!(
        composer
            .renderer()
            .effects()
            .has_program(DefinitionKind::Effect, "blur")
    );

    // An unrelated registration reaches view A as a new set version.
    let extended = Arc::new(with_extra(&builtins, 2));
    let failures = composer.use_definitions(a, Some(&(extended.clone(), 2)), &[]);
    assert_eq!(failures.map(|f| f.len()), Some(0), "the new set loaded");
    let effects = composer.renderer().effects();
    assert!(
        effects.watchdog.is_disabled(DefinitionKind::Effect, "blur"),
        "the watchdog record is kept"
    );
    assert!(
        effects.has_program(DefinitionKind::Effect, "blur"),
        "the program is kept (no pipeline rebuild)"
    );
    let after_push = render(&mut composer, &doc);
    assert!(
        max_diff(&after_push, &base) <= 1.0 / 1024.0,
        "still off after an unrelated push"
    );
    assert!(composer.renderer().watchdog_reports(true).is_empty());

    // View B on the same device draws the blur with its own set; A's state
    // survives the swaps both ways. (Each view's set has its own watchdog;
    // B's draws within budget.)
    composer.use_definitions(b, Some(&(builtins.clone(), 1)), &[]);
    blur_ms.store(5, Ordering::Relaxed);
    composer
        .renderer()
        .effects_mut()
        .watchdog
        .set_timing_source(Some(blur_takes(blur_ms.clone())));
    assert!(
        !composer
            .renderer()
            .effects()
            .watchdog
            .is_disabled(DefinitionKind::Effect, "blur"),
        "view B has its own watchdog"
    );
    let in_b = render(&mut composer, &doc);
    assert!(
        max_diff(&in_b, &first) <= 1.0 / 1024.0,
        "view B draws the blur"
    );
    assert_eq!(
        composer.use_definitions(a, Some(&(extended.clone(), 2)), &[]),
        None,
        "A's set is current: nothing to load"
    );
    let back_in_a = render(&mut composer, &doc);
    assert!(
        max_diff(&back_in_a, &base) <= 1.0 / 1024.0,
        "still off in view A after view B drew"
    );
    assert!(
        composer
            .renderer()
            .effects()
            .has_program(DefinitionKind::Effect, "blur")
    );

    // The blur registered again with the same source (a kernel delta's
    // upsert: a register or hot reload of that id) re-enables it.
    let same = Arc::new(with_extra(&builtins, 3));
    composer.use_definitions(
        a,
        Some(&(same, 3)),
        &[(DefinitionKind::Effect, "blur".into())],
    );
    let registered = render(&mut composer, &doc);
    assert!(
        max_diff(&registered, &first) <= 1.0 / 1024.0,
        "re-enabled by the registration"
    );
    assert!(composer.renderer().watchdog_reports(true).is_empty());
    // Off again for the rest (a fresh overrun, judged at once).
    blur_ms.store(300, Ordering::Relaxed);
    render(&mut composer, &doc);
    assert_eq!(
        composer.renderer().watchdog_reports(true).len(),
        1,
        "overran again"
    );
    assert!(max_diff(&render(&mut composer, &doc), &base) <= 1.0 / 1024.0);

    // A hot reload that changes the blur's source re-enables it.
    let mut reloaded = (*extended).clone();
    reloaded.generation = Seq::new(4).unwrap();
    let blur = reloaded
        .definitions
        .iter_mut()
        .find(|d| d.id.as_str() == "blur")
        .unwrap();
    blur.glsl
        .as_mut()
        .unwrap()
        .source
        .push_str("\n// reloaded\n");
    blur_ms.store(5, Ordering::Relaxed);
    composer.use_definitions(a, Some(&(Arc::new(reloaded), 4)), &[]);
    let again = render(&mut composer, &doc);
    assert!(max_diff(&again, &first) <= 1.0 / 1024.0, "re-enabled");

    // A closed session's set is dropped.
    composer.forget_definitions(a);
    assert!(!composer.renderer().effects().configured());
}
