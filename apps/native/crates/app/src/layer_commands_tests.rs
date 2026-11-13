//! Commands run on the real actor, and the picture is compared on the same
//! export path the viewer renders through.
use super::*;
use pm_kernel::{
    Ack, Call, Caller, HistoryCall, KernelActor, KernelConfig, KernelHandle, OpenSource, Reply,
    Request, SelectionCall, WriteBody,
};
use pm_protocol::{color::Precision, common::Size, common::Ticks};
use std::sync::Arc;

const W: u32 = 96;
const H: u32 = 54;
/// Each renderer starts the shader compiler pool; they run one at a time.
static PIXELS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn layer(id: &str, kind: &str, x: f64, y: f64, rotation: f64, color: &str, extra: &str) -> String {
    format!(
        r##"{{"id":"{id}","type":"{kind}","name":"{id}","from":0,"dur":3,"on":true,"lock":false,
        "parent":null,"group":null,"blend":"normal","mblur":false,"masks":[],
        "p":{{"position.x":{{"v":{x},"kf":[]}},"position.y":{{"v":{y},"kf":[]}},
        "anchor.x":{{"v":0,"kf":[]}},"anchor.y":{{"v":0,"kf":[]}},
        "scale.x":{{"v":100,"kf":[]}},"scale.y":{{"v":100,"kf":[]}},
        "rotation":{{"v":{rotation},"kf":[]}},"opacity":{{"v":100,"kf":[]}}}},
        "d":{{"w":24,"h":12,"color":"{color}"}},"fx":[]{extra}}}"##
    )
}
fn open(layers: &[String]) -> (KernelActor, KernelHandle) {
    open_on(r#"{"type":"none","angle":0,"stops":[]}"#, layers)
}
fn open_on(background: &str, layers: &[String]) -> (KernelActor, KernelHandle) {
    let actor = KernelActor::spawn(KernelConfig::default());
    let project = pm_model::json::parse(&format!(
        r##"{{"id":"synthetic","name":"Commands","compId":"root","w":{W},"h":{H},"fps":30,"dur":3,
        "colorDepth":16,"bg":"#000000","backgroundFill":{background},
        "assets":{{}},"comps":{{}},"layers":[{}]}}"##,
        layers.join(",")
    ))
    .unwrap();
    let kernel = actor.handle();
    kernel
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    (actor, kernel)
}
fn send(kernel: &KernelHandle, call: Call) -> Ack {
    let ack = kernel
        .call("synthetic", Request::new(Caller::native(), call))
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
    ack
}
fn select(kernel: &KernelHandle, ids: &[&str]) -> Arc<Published> {
    send(
        kernel,
        SelectionCall::Layers {
            ids: ids.iter().map(|id| (*id).into()).collect(),
            add: false,
        }
        .into(),
    );
    kernel.published("synthetic").unwrap()
}
fn run(kernel: &KernelHandle, plan: Plan) {
    let revision = kernel.published("synthetic").unwrap().revision();
    send(
        kernel,
        Call::Transact {
            label: Some(plan.label),
            expected_revision: Some(revision),
            body: WriteBody::Commands(plan.commands),
        },
    );
    if let Some(ids) = plan.select {
        send(kernel, SelectionCall::Layers { ids, add: false }.into());
    }
}
fn frame(kernel: &KernelHandle, renderer: &mut pm_render::Renderer, time: f64) -> Vec<f32> {
    let p = kernel.published("synthetic").unwrap();
    renderer
        .set_main_effects_for_document(&p.snapshot.document)
        .unwrap();
    let mut request = pm_render::RenderRequest::sized(Size::new(W, H), Precision::Float16);
    request.export = true;
    request.output_kind = pm_render::OutputKind::Final;
    let frame = pm_export::render::render_frame(
        renderer,
        &p.snapshot.document,
        "root",
        Ticks::new((time * 705_600_000.).round() as i64).unwrap(),
        &request,
        &Default::default(),
        &pm_export::render::Unrestricted,
        1,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    frame.rgba
}
fn same(a: &[f32], b: &[f32], what: &str) {
    assert_eq!(a.len(), b.len());
    assert!(
        a.iter().any(|v| *v > 0.1),
        "{what}: the fixture draws something"
    );
    let worst = a
        .iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0., f32::max);
    assert!(worst <= 2. / 255., "{what}: largest difference {worst}");
}
fn draft(published: &Published, leave: bool) -> PrecomposeDraft {
    PrecomposeDraft {
        leave,
        ..precompose_draft(published).unwrap()
    }
}

#[test]
fn center_anchor_keeps_the_picture_with_undo() {
    let _pixels = PIXELS.lock().unwrap_or_else(|e| e.into_inner());
    let (_actor, kernel) = open(&[layer("a", "solid", 40., 20., 30., "#FF0000", "")]);
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let original = frame(&kernel, &mut renderer, 0.);
    let published = select(&kernel, &["a"]);
    run(&kernel, plan("centerAnchor", &published, 0.).unwrap());
    let moved = kernel.published("synthetic").unwrap();
    let root = moved.snapshot.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, "a").unwrap();
    let centered = pm_kernel::model::layer_at(root, &loc).unwrap().to_value();
    let anchor = centered.get("p").unwrap().get("anchor.x").unwrap();
    assert_eq!(anchor.get("v").and_then(Value::as_f64), Some(12.));
    same(&original, &frame(&kernel, &mut renderer, 0.), "centered");
    send(&kernel, HistoryCall::Undo.into());
    same(&original, &frame(&kernel, &mut renderer, 0.), "undone");
}

#[test]
fn precompose_move_keeps_the_picture_and_releases_back() {
    let _pixels = PIXELS.lock().unwrap_or_else(|e| e.into_inner());
    let (_actor, kernel) = open(&[
        layer("a", "solid", 40., 20., 0., "#FF0000", ""),
        layer("b", "solid", 50., 30., 15., "#00FF00", ""),
        layer("c", "solid", 10., 10., 0., "#0000FF", ""),
    ]);
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let original = frame(&kernel, &mut renderer, 0.);
    let published = select(&kernel, &["a", "b"]);
    let plan_ = precompose_plan(&published, &draft(&published, false), 0.);
    let precomp = plan_.select.clone().unwrap().remove(0);
    run(&kernel, plan_);
    let after = kernel.published("synthetic").unwrap();
    assert_eq!(after.selection.layers(), std::slice::from_ref(&precomp));
    same(
        &original,
        &frame(&kernel, &mut renderer, 0.),
        "pre-composed",
    );
    let released = plan("releasePrecomp", &after, 0.).unwrap();
    run(&kernel, released);
    same(&original, &frame(&kernel, &mut renderer, 0.), "released");
}

#[test]
fn precompose_and_open_is_one_undo_step() {
    let (_actor, kernel) = open(&[layer("a", "solid", 40., 20., 0., "#FF0000", "")]);
    let published = select(&kernel, &["a"]);
    let draft = PrecomposeDraft {
        open_after: true,
        ..precompose_draft(&published).unwrap()
    };
    let open_comp = |kernel: &KernelHandle| {
        let p = kernel.published("synthetic").unwrap();
        let open = p.snapshot.open_comp().unwrap();
        let root = p.snapshot.root().unwrap();
        let exists = root
            .lookup(&["comps".into(), open.as_str().into()])
            .is_some();
        (open, exists)
    };
    let original = open_comp(&kernel);
    run(&kernel, precompose_plan(&published, &draft, 0.));
    let (opened, exists) = open_comp(&kernel);
    assert!(exists && opened != original.0);
    assert!(
        kernel
            .published("synthetic")
            .unwrap()
            .selection
            .layers()
            .is_empty()
    );
    send(&kernel, HistoryCall::Undo.into());
    assert_eq!(open_comp(&kernel), original);
    send(&kernel, HistoryCall::Redo.into());
    assert_eq!(open_comp(&kernel), (opened, true));
}

#[test]
fn precompose_leave_keeps_an_effect_on_the_outer_layer() {
    let _pixels = PIXELS.lock().unwrap_or_else(|e| e.into_inner());
    let fx = r#","fx":[{"id":"f1","type":"invert","on":true,"open":true,"version":2,
        "p":{"channel":{"v":0,"kf":[]},"blendWithOriginal":{"v":0,"kf":[]}}}]"#;
    // Pixel-aligned on purpose: a nested composition's rotated edge is resampled,
    // so its antialiased rim differs from a directly drawn solid's.
    let source = layer("a", "solid", 40., 20., 0., "#2040A0", "").replace(r#","fx":[]"#, fx);
    let (_actor, kernel) = open(&[source]);
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let original = frame(&kernel, &mut renderer, 0.);
    let published = select(&kernel, &["a"]);
    let draft = draft(&published, true);
    assert!(draft.can_leave);
    run(&kernel, precompose_plan(&published, &draft, 0.));
    let p = kernel.published("synthetic").unwrap();
    let root = p.snapshot.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, "a").unwrap();
    let kept = pm_kernel::model::layer_at(root, &loc).unwrap().to_value();
    assert_eq!(kind(&kept), Some("precomp"));
    assert_eq!(kept.get("fx").unwrap().as_array().unwrap().len(), 1);
    same(
        &original,
        &frame(&kernel, &mut renderer, 0.),
        "leave attributes",
    );
}

#[test]
fn toggle_continuous_raster_turns_every_selected_precomp_on_then_off() {
    let sub = r##""d":{"comp":"sub","w":24,"h":12}"##;
    let (_actor, kernel) = open(&[
        layer("a", "precomp", 40., 20., 0., "", "")
            .replace(r##""d":{"w":24,"h":12,"color":""}"##, sub),
        layer("b", "solid", 40., 20., 0., "#FF0000", ""),
    ]);
    let flag = |kernel: &KernelHandle| {
        let p = kernel.published("synthetic").unwrap();
        let root = p.snapshot.root().unwrap();
        let loc = pm_kernel::model::find_layer(root, "a").unwrap();
        pm_kernel::model::layer_at(root, &loc)
            .unwrap()
            .to_value()
            .get("continuousRaster")
            .and_then(Value::as_bool)
    };
    let published = select(&kernel, &["a", "b"]);
    run(
        &kernel,
        plan("toggleContinuousRaster", &published, 0.).unwrap(),
    );
    assert_eq!(flag(&kernel), Some(true));
    let published = kernel.published("synthetic").unwrap();
    run(
        &kernel,
        plan("toggleContinuousRaster", &published, 0.).unwrap(),
    );
    assert_eq!(flag(&kernel), Some(false));
}

#[test]
fn commands_with_nothing_to_act_on_are_not_handled() {
    let (_actor, kernel) = open(&[layer("a", "solid", 40., 20., 0., "#FF0000", "")]);
    let none = kernel.published("synthetic").unwrap();
    assert!(plan("centerAnchor", &none, 0.).is_none());
    assert!(precompose_draft(&none).is_none());
    let published = select(&kernel, &["a"]);
    for name in [
        "releasePrecomp",
        "toggleContinuousRaster",
        "splitTextWords",
        "other",
    ] {
        assert!(plan(name, &published, 0.).is_none(), "{name}");
    }
}

#[test]
fn a_font_registry_that_cannot_answer_refuses_the_split_and_writes_nothing() {
    let broker = pm_text::host::FontBroker::new();
    let fonts = pm_render::text::host::RenderFonts::new(broker.clone());
    broker.close();
    let actor = KernelActor::spawn(KernelConfig {
        text: Some(crate::text_host::text_host(&fonts)),
        ..Default::default()
    });
    let kernel = actor.handle();
    let source = layer("t", "text", 20., 30., 0., "#FFFFFF", "");
    let project = pm_model::json::parse(&format!(
        r##"{{"id":"synthetic","name":"Text","compId":"root","w":{W},"h":{H},"fps":30,"dur":3,
        "bg":"#000000","assets":{{}},"comps":{{}},"layers":[{source}]}}"##
    ))
    .unwrap();
    kernel
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let published = select(&kernel, &["t"]);
    let before = published.revision();
    let plan = plan("splitTextWords", &published, 0.).unwrap();
    let ack = kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some(plan.label),
                    expected_revision: Some(before),
                    body: WriteBody::Commands(plan.commands),
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
    assert_eq!(kernel.published("synthetic").unwrap().revision(), before);
}

/// What the viewer draws: the engine service's own frame path, not a test
/// renderer. Every seeded definition applies at its defaults and draws.
#[test]
fn every_seeded_effect_adds_through_the_kernel_and_draws_in_the_viewer_path() {
    let _pixels = PIXELS.lock().unwrap_or_else(|e| e.into_inner());
    // A grey ground: a black shadow has to be visible beside the solid.
    let (_actor, kernel) = open_on(
        r##"{"type":"solid","color":"#808080","angle":0,"stops":[]}"##,
        &[layer("a", "solid", 40., 20., 0., "#A04020", "")],
    );
    let mut headless = pm_engine_service::Headless::new().unwrap();
    let size = Size::new(W, H);
    let draw = |headless: &mut pm_engine_service::Headless, kernel: &KernelHandle| {
        let p = kernel.published("synthetic").unwrap();
        let scene = pm_engine_service::Scene {
            doc: p.snapshot.document.clone(),
            comp: "root".into(),
            media: Default::default(),
            content: p.revision(),
        };
        headless
            .frame(&scene, Ticks::ZERO, size, &Default::default())
            .unwrap()
    };
    let plain = draw(&mut headless, &kernel);
    assert!(plain.iter().any(|p| p[0] > 0.1), "the fixture draws");
    let state = kernel.state();
    let seeded = state.registries.get("effects").unwrap();
    assert_eq!(seeded.len(), 43);
    let mut changed = Vec::new();
    for entry in seeded {
        let p = kernel.published("synthetic").unwrap();
        let version = entry.definition.get("version").cloned();
        let definition = pm_kernel::commands::effects::saved_builtin_definition(
            &p.snapshot.document,
            &entry.id,
            version.as_ref(),
            24.,
            12.,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", entry.id));
        let effect_id = new_id("f");
        run(
            &kernel,
            Plan {
                count_keys: None,
                label: "Add Effect".into(),
                commands: vec![
                    pm_kernel::EffectCommand::Add(Box::new(
                        pm_kernel::commands::effects::AddEffect {
                            layer_id: "a".into(),
                            effect_id: effect_id.clone(),
                            definition,
                            parameters: Default::default(),
                            inputs: None,
                            open: true,
                            enabled: true,
                            enabled_animation: None,
                            index: None,
                        },
                    ))
                    .into(),
                ],
                select: None,
            },
        );
        if draw(&mut headless, &kernel) != plain {
            changed.push(entry.id.clone());
        }
        send(&kernel, HistoryCall::Undo.into());
    }
    // Definitions whose defaults are not an identity on a flat, opaque solid.
    for id in [
        "motionblurDir",
        "sharpen",
        "duotone",
        "grain",
        "chroma",
        "pixelate",
        "posterize",
        "displace",
        "shadow",
        "invert",
        "gradient",
        "tint",
        "threshold",
        "findEdges",
        "tritone",
        "blackWhite",
        "channelCombiner",
        "hdrCompander",
    ] {
        assert!(
            changed.iter().any(|c| c == id),
            "{id} drew nothing: {changed:?}"
        );
    }
}

/// The session's nudge burst: a mark, one write per arrow key, and a squash
/// once the keys rest leave one "Nudge Selection" undo step.
#[test]
fn a_burst_of_nudges_squashes_into_one_undo_step() {
    let (_actor, kernel) = open(&[layer("a", "solid", 40., 20., 0., "#FF0000", "")]);
    let x = |kernel: &KernelHandle| {
        let p = kernel.published("synthetic").unwrap();
        let root = p.snapshot.root().unwrap();
        let loc = pm_kernel::model::find_layer(root, "a").unwrap();
        let layer = pm_kernel::model::layer_at(root, &loc).unwrap().to_value();
        layer
            .get("p")
            .unwrap()
            .get("position.x")
            .unwrap()
            .get("v")
            .and_then(Value::as_f64)
    };
    select(&kernel, &["a"]);
    let Reply::Mark(mark) = send(&kernel, HistoryCall::Mark.into()).reply else {
        panic!("a history mark");
    };
    for _ in 0..3 {
        let published = kernel.published("synthetic").unwrap();
        let step = [serde_json::json!(1), serde_json::json!(0)];
        run(
            &kernel,
            command_plan("nudgeSelection", &step, &published, 0.).unwrap(),
        );
    }
    assert_eq!(x(&kernel), Some(43.));
    let expected_revision = kernel.published("synthetic").unwrap().revision();
    let squash = HistoryCall::Squash {
        mark,
        label: "Nudge Selection".into(),
        expected_revision,
    };
    assert!(matches!(
        send(&kernel, squash.into()).reply,
        Reply::Entry(Some(_))
    ));
    send(&kernel, HistoryCall::Undo.into());
    assert_eq!(x(&kernel), Some(40.), "one undo restores the burst");
}

#[test]
fn project_commands_wait_for_an_arriving_project_and_others_do_not() {
    use crate::layer_commands::waits_for_project;
    for name in [
        "compositionSettings",
        "newComposition",
        "export",
        "precompose",
        "delete",
    ] {
        assert!(waits_for_project(name), "{name}");
    }
    for name in [
        "settings",
        "agent",
        "quit",
        "newProject",
        "openRecent",
        "toolSelect",
    ] {
        assert!(!waits_for_project(name), "{name}");
    }
}
