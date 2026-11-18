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
    // 43 shipped definitions plus Color Wheels, Color Curves and Color Space
    // Transform (whose defaults are an identity too).
    assert_eq!(seeded.len(), 46);
    let mut changed = Vec::new();
    let mut failed = Vec::new();
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
        } else if let Some(failure) = headless.definition_failure() {
            failed.push(format!("{}: {failure}", entry.id));
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
            "{id} drew nothing: {changed:?}; failures: {failed:?}"
        );
    }
    // A neutral grade is an identity: the grading tools at their defaults
    // leave the picture exactly as it was.
    for id in ["colorWheels", "colorCurves"] {
        assert!(
            !changed.iter().any(|c| c == id),
            "{id} changed the picture at its defaults"
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

/// Arrow keys move a 3D layer in scene units, up is up: 0.1 a press, 1 with ⇧.
#[test]
fn a_nudge_moves_a_3d_layer_in_scene_units() {
    let model = r##"{"id":"m","type":"extension","name":"Cube","from":0,"dur":3,"on":true,
        "lock":false,"parent":null,"group":null,"blend":"normal","mblur":false,"masks":[],
        "p":{"position.x":{"v":0.3,"kf":[]},"position.y":{"v":0,"kf":[]},"position.z":{"v":0,"kf":[]}},
        "d":{"definition":"powermove.3d.object","data":{"object":{"source":{"primitive":"box"}}}},
        "fx":[]}"##;
    let (_actor, kernel) = open(&[model.to_string()]);
    let value = |kernel: &KernelHandle, channel: &str| {
        let p = kernel.published("synthetic").unwrap();
        let root = p.snapshot.root().unwrap();
        let loc = pm_kernel::model::find_layer(root, "m").unwrap();
        let layer = pm_kernel::model::layer_at(root, &loc).unwrap().to_value();
        layer
            .get("p")
            .and_then(|p| p.get(channel))
            .and_then(|c| c.get("v"))
            .and_then(Value::as_f64)
    };
    select(&kernel, &["m"]);
    for step in [[1., 0.], [0., -10.]] {
        let published = kernel.published("synthetic").unwrap();
        let step = step.map(|v| serde_json::json!(v));
        run(
            &kernel,
            command_plan("nudgeSelection", &step, &published, 0.).unwrap(),
        );
    }
    assert_eq!(value(&kernel, "position.x"), Some(0.4));
    assert_eq!(value(&kernel, "position.y"), Some(1.));
}

/// Properties' Parent and Track Matte pop-ups list their rows when they
/// open (`host_rows`), read on the main thread from the publication: the
/// same layers the kernel's relation questions allow, asked one layer at a
/// time, and the selection a layer belongs to is parented as one.
#[test]
fn host_rows_list_the_layers_a_layer_can_follow() {
    let parented = |id: &str, parent: &str| {
        layer(id, "solid", 0., 0., 0., "#ffffff", "")
            .replace(r#""parent":null"#, &format!(r#""parent":"{parent}""#))
    };
    let (_actor, kernel) = open(&[
        layer("a", "solid", 0., 0., 0., "#ffffff", ""),
        parented("b", "a"),
        parented("c", "b"),
        layer("d", "audio", 0., 0., 0., "#ffffff", ""),
        layer("e", "solid", 0., 0., 0., "#ffffff", ""),
    ]);
    let ids = |rows: Vec<serde_json::Value>| -> Vec<Option<String>> {
        rows.iter()
            .map(|r| r["v"].as_str().map(str::to_owned))
            .collect()
    };
    let parents = |p: &Published, layer: &str| {
        ids(host_rows(
            p,
            &serde_json::json!({"source": "layer-parents", "layer": layer}),
        )
        .unwrap())
    };
    let s = |v: &str| Some(v.to_owned());
    let p = select(&kernel, &[]);
    // Not itself, not audio, nothing that would make a cycle.
    assert_eq!(parents(&p, "a"), [None, s("e")]);
    assert_eq!(parents(&p, "c"), [None, s("a"), s("b"), s("e")]);
    // A selected layer is parented with the rest of its selection.
    let p = select(&kernel, &["b", "c"]);
    assert_eq!(parents(&p, "c"), [None, s("a"), s("e")]);
    // The same answers as asking the kernel layer by layer.
    let facts = Facts::read(&p, None).unwrap();
    let children: Vec<&LayerInfo> = facts
        .layers
        .iter()
        .filter(|l| l.id == "b" || l.id == "c")
        .collect();
    let slow: Vec<Option<String>> = std::iter::once(None)
        .chain(
            parent_candidates(&p, &facts, &children)
                .iter()
                .map(|l| s(&l.id)),
        )
        .collect();
    assert_eq!(parents(&p, "c"), slow);
    for layer in ["a", "b", "c", "e"] {
        let mattes = ids(host_rows(
            &p,
            &serde_json::json!({"source": "layer-mattes", "layer": layer}),
        )
        .unwrap());
        let mut mirror = pm_kernel::QueryMirror::new(&p);
        let slow: Vec<Option<String>> = std::iter::once(None)
            .chain(
                facts
                    .layers
                    .iter()
                    .filter(|l| relation(&mut mirror, &p, "validMatteSource", layer, ("id", &l.id)))
                    .map(|l| s(&l.id)),
            )
            .collect();
        assert_eq!(mattes, slow, "Track Matte rows for {layer}");
    }
    // An unknown source or layer lists nothing.
    assert!(host_rows(&p, &serde_json::json!({"source": "nope", "layer": "a"})).is_none());
    assert!(
        host_rows(
            &p,
            &serde_json::json!({"source": "layer-mattes", "layer": "zz"})
        )
        .is_none()
    );
}
/// Acceptance run 27 (PM-502): two layers named "Rect" looked the same in
/// the Track Matte list. Rows read by stack number and name, as After
/// Effects lists them.
#[test]
fn track_matte_rows_tell_layers_of_one_name_apart() {
    let rect = |id: &str| {
        layer(id, "shape", 0., 0., 0., "#ffffff", "")
            .replace(&format!(r#""name":"{id}""#), r#""name":"Rect""#)
    };
    let (_actor, kernel) = open(&[
        layer("t", "solid", 0., 0., 0., "#ffffff", ""),
        rect("r1"),
        rect("r2"),
    ]);
    let p = select(&kernel, &[]);
    let rows = host_rows(
        &p,
        &serde_json::json!({"source": "layer-parents", "layer": "t"}),
    )
    .unwrap();
    let labels: Vec<&str> = rows.iter().filter_map(|r| r["label"].as_str()).collect();
    assert_eq!(labels, ["None", "2. Rect", "3. Rect"]);
}

#[test]
fn paired_crossfade_is_not_a_dip_and_preview_matches_export() {
    let _guard = PIXELS.lock().unwrap();
    // Reverse the stack deliberately: B must be above A only while rendering
    // their paired overlap, regardless of stored document order.
    let make = |id: &str, color: &str, start: f64| {
        let mut clip: serde_json::Value =
            serde_json::from_str(&layer(id, "solid", 0., 0., 0., color, "")).unwrap();
        clip["from"] = serde_json::json!(start);
        clip["dur"] = serde_json::json!(1.5);
        clip["track"] = serde_json::json!(0);
        clip.to_string()
    };
    let (_actor, kernel) = open(&[make("a", "#FF0000", 0.), make("b", "#0000FF", 1.5)]);
    let published = select(&kernel, &["a"]);
    let commands = pm_kernel::commands::transition_edges::plan(
        published.snapshot.root().unwrap(),
        "a",
        pm_kernel::commands::effects::Edge::Out,
        Some(pm_kernel::commands::effects::TransitionSpec {
            definition: pm_kernel::commands::effects::TransitionDefinition {
                transition_type: "crossfade".into(),
                params: vec![],
            },
            duration: Some(0.5),
            params: Default::default(),
        }),
        true,
    )
    .unwrap();
    send(
        &kernel,
        Call::Transact {
            label: Some("Add Transition".into()),
            expected_revision: Some(published.revision()),
            body: WriteBody::Commands(commands),
        },
    );
    let mut renderer = pm_render::Renderer::headless().unwrap();
    for (time, expected) in [
        (1.25, [1., 0., 0., 1.]),
        (1.5, [0.5, 0., 0.5, 1.]),
        (1.75, [0., 0., 1., 1.]),
    ] {
        let rgba = frame(&kernel, &mut renderer, time);
        let pixel = &rgba[(4 * W as usize + 4) * 4..][..4];
        for (got, expected) in pixel.iter().zip(expected) {
            assert!((got - expected).abs() < 0.01, "at {time}: {pixel:?}");
        }
        let publication = kernel.published("synthetic").unwrap();
        let mut request = pm_render::RenderRequest::sized(Size::new(W, H), Precision::Float16);
        request.export = false;
        let preview = renderer
            .render(
                &publication.snapshot.document,
                "root",
                Ticks::new((time * 705_600_000.).round() as i64).unwrap(),
                &request,
                &pm_render::MapResolver::default(),
            )
            .unwrap();
        same(
            &renderer.read_rgba(&preview).unwrap(),
            &rgba,
            "paired preview and export",
        );
    }
    let saved = pm_model::json::to_canonical_string(
        &kernel
            .published("synthetic")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .to_value(),
    );
    assert!(kernel.close("synthetic").wait().unwrap());
    kernel
        .open(
            "synthetic",
            OpenSource::Untitled(pm_model::json::parse(&saved).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    for (time, expected) in [
        (1.25, [1., 0., 0., 1.]),
        (1.5, [0.5, 0., 0.5, 1.]),
        (1.75, [0., 0., 1., 1.]),
    ] {
        let rgba = frame(&kernel, &mut renderer, time);
        let pixel = &rgba[(4 * W as usize + 4) * 4..][..4];
        for (got, want) in pixel.iter().zip(expected) {
            assert!(
                (got - want).abs() < 0.01,
                "reopened pair at {time}: {pixel:?}"
            );
        }
    }
}

#[test]
fn paired_crossfade_handle_shortfalls_render_without_a_dip_or_held_frames() {
    let _guard = PIXELS.lock().unwrap();
    // Finite precomps exercise actual source-time evaluation without decoding
    // a video fixture. Their visible content ends at the media boundary.
    for (tail, head, duration, offset) in [
        (0.1, 0.4, 0.2, -0.1),
        (0., 0.4, 0.4, -0.4),
        (0.4, 0., 0.4, 0.),
    ] {
        let (_actor, kernel) = cut_with_handles(tail, head);
        let mut renderer = pm_render::Renderer::headless().unwrap();
        for (progress, expected) in [
            (0., [1., 0., 0., 1.]),
            (0.5, [0.5, 0., 0.5, 1.]),
            (1., [0., 0., 1., 1.]),
        ] {
            let time = 1.5 + offset + duration * progress;
            let rgba = frame(&kernel, &mut renderer, time);
            let pixel = &rgba[(4 * W as usize + 4) * 4..][..4];
            for (got, expected) in pixel.iter().zip(expected) {
                assert!(
                    (got - expected).abs() < 0.01,
                    "tail={tail} head={head} t={time}: {pixel:?}"
                );
            }
        }
    }
}

fn cut_with_handles(tail: f64, head: f64) -> (KernelActor, KernelHandle) {
    let mut clips = Vec::new();
    let mut comps = serde_json::Map::new();
    for (id, color, from, trim, source_duration) in [
        ("a", "#FF0000", 0., 0., 1.5 + tail),
        ("b", "#0000FF", 1.5, head, 1.5 + head),
    ] {
        let mut content: serde_json::Value = serde_json::from_str(&layer(
            &format!("{id}-content"),
            "solid",
            0.,
            0.,
            0.,
            color,
            "",
        ))
        .unwrap();
        content["dur"] = serde_json::json!(source_duration);
        comps.insert(
            id.into(),
            serde_json::json!({
                "id":id,"w":W,"h":H,"fps":30,"dur":source_duration,
                "backgroundFill":{"type":"none","angle":0,"stops":[]},"layers":[content]
            }),
        );
        let mut clip: serde_json::Value =
            serde_json::from_str(&layer(id, "precomp", 0., 0., 0., "", "")).unwrap();
        clip["from"] = serde_json::json!(from);
        clip["dur"] = serde_json::json!(1.5);
        clip["track"] = serde_json::json!(0);
        clip["d"] = serde_json::json!({"comp":id,"w":W,"h":H,"trim":trim});
        clips.push(clip);
    }
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let project = serde_json::json!({
        "id":"synthetic","name":"Handles","compId":"root","w":W,"h":H,"fps":30,"dur":3,
        "colorDepth":16,"backgroundFill":{"type":"none","angle":0,"stops":[]},
        "assets":{},"comps":comps,"layers":clips
    });
    kernel
        .open(
            "synthetic",
            OpenSource::Untitled(pm_model::json::parse(&project.to_string()).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    let published = kernel.published("synthetic").unwrap();
    let commands = pm_kernel::commands::transition_edges::plan(
        published.snapshot.root().unwrap(),
        "a",
        pm_kernel::commands::effects::Edge::Out,
        pm_kernel::commands::transition_edges::builtin_spec("crossfade"),
        true,
    )
    .unwrap();
    send(
        &kernel,
        Call::Transact {
            label: Some("Add Transition".into()),
            expected_revision: Some(published.revision()),
            body: WriteBody::Commands(commands),
        },
    );
    (actor, kernel)
}

#[test]
fn g2_transition_notice_distinguishes_shortening_from_a_real_shift() {
    for (tail, head, expected) in [
        (
            0.1,
            0.4,
            Some("Transition shortened to fit available source media (0.20 s)"),
        ),
        (
            0.,
            0.4,
            Some("Transition shortened and shifted to fit available source media (0.40 s)"),
        ),
        (
            0.4,
            0.,
            Some("Transition shortened and shifted to fit available source media (0.40 s)"),
        ),
        (0.3, 0.3, None),
    ] {
        let (_actor, kernel) = cut_with_handles(tail, head);
        let published = kernel.published("synthetic").unwrap();
        assert_eq!(
            crate::layer_commands::transition_notice(&published, &[("a".into(), 0.5)]).as_deref(),
            expected,
        );
    }
}

#[test]
fn paired_crossfade_roll_refits_render_and_capsule_and_undo_restores_intent() {
    use pm_kernel::commands::timing::{TimeEdge, TimingCommand};
    use pm_panel_timeline::render as paint;
    struct Presenter;
    impl paint::ModelPresenter for Presenter {
        fn accessibility_label(&self, _: &pm_panel_timeline::RowId) -> String {
            "Clip".into()
        }
        fn layer_label(&self, _: &pm_panel_timeline::Layer) -> paint::Label {
            paint::Label::for_layer("precomp", None)
        }
        fn media(&self, _: &pm_panel_timeline::Layer) -> paint::Media {
            paint::Media::Ready
        }
        fn interpolation(&self, _: &pm_panel_timeline::Key) -> paint::Interpolation {
            paint::Interpolation::Linear
        }
        fn track_accessibility_label(
            &self,
            _: pm_panel_timeline::tracks::Area,
            _: usize,
        ) -> Option<String> {
            Some("Video 1".into())
        }
    }
    let _guard = PIXELS.lock().unwrap();
    let (_actor, kernel) = cut_with_handles(0.3, 0.3);
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let tokens = pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default());
    let check = |renderer: &mut pm_render::Renderer, cut: f64, effective: f64, shortened: bool| {
        let p = kernel.published("synthetic").unwrap();
        let root = p.snapshot.root().unwrap();
        let a = pm_kernel::model::layer_at(root, &pm_kernel::model::find_layer(root, "a").unwrap())
            .unwrap();
        assert_eq!(
            a.get("transitionOut")
                .unwrap()
                .get("dur")
                .unwrap()
                .to_value()
                .as_f64(),
            Some(0.5)
        );
        assert_eq!(
            a.get("transitionOut")
                .unwrap()
                .get("offset")
                .unwrap()
                .to_value()
                .as_f64(),
            Some(-0.25)
        );
        let mut model = pm_panel_timeline::TimelineModel::default();
        model.sync(&p);
        model.mode = pm_panel_timeline::TimelineMode::Tracks;
        model.rebuild_rows();
        let view = paint::Viewport {
            width: 1000.,
            height: 250.,
            scroll_y: 0.,
            scroll_time: 0.,
            pps: 200.,
            layer_column: 200.,
        };
        let timeline = paint::Scene::from_model(&model, &Presenter, &tokens, cut)
            .unwrap()
            .prepare(view, &tokens)
            .unwrap();
        let capsule = timeline
            .ops
            .iter()
            .find(|op| op.id == "transition:b:in")
            .unwrap();
        assert!((capsule.detail["effectiveDuration"].as_f64().unwrap() - effective).abs() < 1e-6);
        let start = cut - effective / 2.;
        if effective > 0. {
            assert!((capsule.bounds.x - (200. + start * 200.)).abs() < 1e-6);
        }
        assert_eq!(capsule.detail["disabled"], effective == 0.);
        assert_eq!(
            timeline
                .tooltip_at(capsule.bounds.x + 1., capsule.bounds.y + 1.)
                .as_deref(),
            if shortened {
                Some("Shortened to fit media")
            } else {
                None
            }
        );
        let samples = if effective > 0. {
            vec![
                (start, [1., 0., 0., 1.]),
                (cut, [0.5, 0., 0.5, 1.]),
                (cut + effective / 2. + 0.01, [0., 0., 1., 1.]),
            ]
        } else {
            vec![
                (cut - 0.01, [1., 0., 0., 1.]),
                (cut + 0.01, [0., 0., 1., 1.]),
            ]
        };
        for (time, expected) in samples {
            let export = frame(&kernel, renderer, time);
            let pixel = &export[(4 * W as usize + 4) * 4..][..4];
            for (got, want) in pixel.iter().zip(expected) {
                assert!(
                    (got - want).abs() < 0.01,
                    "cut={cut} effective={effective} t={time}: {pixel:?}"
                );
            }
            let mut request = pm_render::RenderRequest::sized(Size::new(W, H), Precision::Float16);
            request.export = false;
            let preview = renderer
                .render(
                    &p.snapshot.document,
                    "root",
                    Ticks::new((time * 705_600_000.).round() as i64).unwrap(),
                    &request,
                    &pm_render::MapResolver::default(),
                )
                .unwrap();
            same(
                &renderer.read_rgba(&preview).unwrap(),
                &export,
                "refitted preview and export",
            );
        }
    };
    check(&mut renderer, 1.5, 0.5, false);
    send(
        &kernel,
        Call::Transact {
            label: Some("Roll Edit".into()),
            expected_revision: None,
            body: WriteBody::Commands(vec![
                TimingCommand::Roll {
                    layer_id: "a".into(),
                    edge: TimeEdge::Out,
                    time: 1.7,
                }
                .into(),
            ]),
        },
    );
    check(&mut renderer, 1.7, 0.2, true);
    send(&kernel, HistoryCall::Undo.into());
    check(&mut renderer, 1.5, 0.5, false);
    send(
        &kernel,
        Call::Transact {
            label: Some("Consume Source Handles".into()),
            expected_revision: None,
            body: WriteBody::Commands(vec![
                TimingCommand::Roll {
                    layer_id: "a".into(),
                    edge: TimeEdge::Out,
                    time: 1.8,
                }
                .into(),
                pm_kernel::PropertyCommand::Set(pm_kernel::commands::properties::SetProperty::new(
                    "b", "c.trim", 0., 1.8,
                ))
                .into(),
            ]),
        },
    );
    check(&mut renderer, 1.8, 0., true);
    send(&kernel, HistoryCall::Undo.into());
    check(&mut renderer, 1.5, 0.5, false);
}

#[test]
fn paired_crossfade_short_clip_can_overlap_both_neighbours() {
    let _guard = PIXELS.lock().unwrap();
    let clip = |id: &str, color: &str, from: f64, duration: f64| {
        let mut value: serde_json::Value =
            serde_json::from_str(&layer(id, "solid", 0., 0., 0., color, "")).unwrap();
        value["from"] = serde_json::json!(from);
        value["dur"] = serde_json::json!(duration);
        value["track"] = serde_json::json!(0);
        value.to_string()
    };
    let (_actor, kernel) = open(&[
        clip("a", "#FF0000", 0., 1.5),
        clip("c", "#0000FF", 1.6, 1.4),
        clip("b", "#00FF00", 1.5, 0.1),
    ]);
    for (left, right) in [("a", "b"), ("b", "c")] {
        send(
            &kernel,
            Call::Transact {
                label: Some("Add Transition".into()),
                expected_revision: None,
                body: WriteBody::Commands(vec![
                    pm_kernel::EffectCommand::SetPairedTransition {
                        left: left.into(),
                        right: right.into(),
                        offset: None,
                        transition: pm_kernel::commands::transition_edges::builtin_spec(
                            "crossfade",
                        )
                        .map(Box::new),
                    }
                    .into(),
                ]),
            },
        );
    }
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let rgba = frame(&kernel, &mut renderer, 1.55);
    let pixel = &rgba[(4 * W as usize + 4) * 4..][..4];
    for (got, want) in pixel.iter().zip([0.1875, 0.5625, 0.25, 1.]) {
        assert!((got - want).abs() < 0.01, "overlapping cuts: {pixel:?}");
    }
}
