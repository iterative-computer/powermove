//! E07 application acceptance uses the real registry, v4 worker and actor.
use super::*;

fn setup() -> (KernelActor, data::Backend) {
    setup_with_saved_effect(None)
}
fn setup_with_saved_effect(saved: Option<Value>) -> (KernelActor, data::Backend) {
    setup_project(|value| {
        if let Some(saved) = saved {
            value["layers"][0]["fx"] = json!([saved]);
        }
    })
}
fn setup_project(edit: impl FnOnce(&mut Value)) -> (KernelActor, data::Backend) {
    let actor = KernelActor::spawn(KernelConfig::default());
    let mut value = json!({"id":"synthetic","name":"Effect contract","compId":"synthetic-root","w":160,"h":90,"fps":30,"dur":3,"bg":"#000000","backgroundFill":{"type":"none","angle":0,"stops":[]},"colorDepth":16,"assets":{},"comps":{},"layers":[{"id":"a","type":"solid","name":"Owner","from":0,"dur":3,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[]},"position.x":{"v":0,"kf":[]},"position.y":{"v":0,"kf":[]},"anchor.x":{"v":0,"kf":[]},"anchor.y":{"v":0,"kf":[]},"scale.x":{"v":100,"kf":[]},"scale.y":{"v":100,"kf":[]},"rotation":{"v":0,"kf":[]},"skew":{"v":0,"kf":[]}},"d":{"w":80,"h":40,"color":"#FFFFFF"},"fx":[]}]});
    edit(&mut value);
    let kernel = actor.handle();
    kernel
        .open(
            "synthetic",
            OpenSource::Untitled(data::model(&value).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    let catalog: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../engine/crates/pm-fx-defs/src/main-effects.json"
    )))
    .unwrap();
    for id in ["gradient", "tint"] {
        let definition = catalog["effects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .unwrap();
        kernel
            .state_transaction(pm_kernel::state::StateTransaction {
                expected_revision: None,
                ops: vec![pm_kernel::state::StateOp::Register {
                    registry: "effects".into(),
                    entry: pm_kernel::state::Registration {
                        token: format!("test-effect-{id}"),
                        owner: Caller::native().writer,
                        id: id.into(),
                        definition: data::model(definition).unwrap(),
                    },
                }],
            })
            .wait()
            .unwrap()
            .unwrap();
    }
    select_layer(&kernel, "a");
    let mut backend = data::Backend::new(
        kernel.clone(),
        pm_surface::host_api::Identity {
            realm: "effect-contract".into(),
            generation: 1,
            project: "synthetic".into(),
        },
        "layer-effects",
        false,
    )
    .unwrap();
    backend.sync().unwrap();
    (actor, backend)
}
fn effect(kernel: &pm_kernel::KernelHandle) -> Value {
    let p = kernel.published("synthetic").unwrap();
    let root = p.snapshot.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, "a").unwrap();
    data::wire(
        &pm_kernel::model::layer_at(root, &loc)
            .unwrap()
            .get("fx")
            .unwrap()
            .to_value(),
    )[0]
    .clone()
}
#[test]
fn e07_application_add_ramp_resolves_source_dimensions_and_keeps_defaults_on_undo_redo() {
    let (actor, mut backend) = setup();
    let kernel = actor.handle();
    let request = backend
        .model
        .as_mut()
        .unwrap()
        .add_effect_at(Caller::native(), "a", "gradient", 0.)
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let created = effect(&kernel);
    assert_eq!(created["version"], 2);
    for (key, expected) in [
        ("startX", 40.),
        ("endX", 40.),
        ("startY", 0.),
        ("endY", 40.),
    ] {
        assert_eq!(
            created["p"][key]["v"], expected,
            "source-sized default {key}"
        );
    }
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    let p = kernel.published("synthetic").unwrap();
    let root = p.snapshot.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, "a").unwrap();
    assert!(
        pm_kernel::model::layer_at(root, &loc)
            .unwrap()
            .get("fx")
            .unwrap()
            .to_value()
            .as_array()
            .unwrap()
            .is_empty()
    );
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Redo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(effect(&kernel), created);
}
#[test]
fn e07_e05_application_declared_color_swaps_are_real_actions_and_undo_atomically() {
    for (id, first, second) in [
        ("gradient", "startColor", "endColor"),
        ("tint", "mapBlackTo", "mapWhiteTo"),
    ] {
        let (actor, mut backend) = setup();
        let kernel = actor.handle();
        let request = backend
            .model
            .as_mut()
            .unwrap()
            .add_effect_at(Caller::native(), "a", id, 0.)
            .unwrap();
        kernel.call("synthetic", request).wait().unwrap().unwrap();
        let before = effect(&kernel);
        let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
        finish(panels.mount("layer-effects").unwrap());
        let publication = panels.take_publications().pop().unwrap();
        let button = publication
            .surface
            .nodes()
            .values()
            .find(|n| n.props.get("label").and_then(Value::as_str) == Some("Swap Colors"))
            .expect("winning definition's declared color action must be exposed");
        finish(
            panels
                .event(
                    "layer-effects",
                    publication.generation,
                    publication.surface.revision(),
                    button.id,
                    "click",
                    EventDetail::default(),
                )
                .unwrap(),
        );
        let swapped = await_effect(&mut panels, &kernel, |value| {
            value["p"][first]["v"] == before["p"][second]["v"]
                && value["p"][second]["v"] == before["p"][first]["v"]
        });
        assert_eq!(
            swapped["p"][first]["v"], before["p"][second]["v"],
            "{id} first color"
        );
        assert_eq!(
            swapped["p"][second]["v"], before["p"][first]["v"],
            "{id} second color"
        );
        kernel
            .call(
                "synthetic",
                Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
            )
            .wait()
            .unwrap()
            .unwrap();
        assert_eq!(
            effect(&kernel),
            before,
            "{id}: one Undo restores both colors"
        );
        kernel
            .call(
                "synthetic",
                Request::new(Caller::native(), pm_kernel::HistoryCall::Redo),
            )
            .wait()
            .unwrap()
            .unwrap();
        assert_eq!(effect(&kernel), swapped);
    }
}

#[test]
fn e07_application_reset_at_playhead_keeps_animation_and_is_one_undo_step() {
    use pm_panel_inspector::{Binding, PropertyAction, PropertyId};
    let (actor, mut backend) = setup();
    let kernel = actor.handle();
    let request = backend
        .model
        .as_mut()
        .unwrap()
        .add_effect_at(Caller::native(), "a", "gradient", 0.)
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let id = effect(&kernel)["id"].as_str().unwrap().to_owned();
    let property = PropertyId::new("a", format!("{id}.startX"));
    for (time, value) in [(0., 10.), (2., 30.)] {
        let model = backend.model.as_mut().unwrap();
        model.sync(kernel.published("synthetic").unwrap());
        let request = model
            .property_action(
                Caller::native(),
                std::slice::from_ref(&property),
                PropertyAction::AddKey,
                time,
            )
            .unwrap();
        kernel.call("synthetic", request).wait().unwrap().unwrap();
        model.sync(kernel.published("synthetic").unwrap());
        let request = model
            .set(
                Caller::native(),
                Binding::Property(property.clone()),
                pm_model::Value::Number(value),
                time,
            )
            .unwrap();
        kernel.call("synthetic", request).wait().unwrap().unwrap();
    }
    let before = effect(&kernel);
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    panels.update_facts(json!({"time":1.,"playing":false,"stage":[160.,90.]}));
    finish(panels.mount("layer-effects").unwrap());
    let projection = panels
        .broker
        .entry("layer-effects")
        .unwrap()
        .lock()
        .unwrap()
        .backend
        .clone()
        .lock()
        .unwrap()
        .projection()
        .unwrap();
    let args = json!({"documentId":projection["documentId"],"revision":projection["revision"],"layer":"a","intent":{"type":"reset","id":id}});
    finish(
        panels
            .realms
            .lock()
            .unwrap()
            .get("layer-effects")
            .unwrap()
            .eval(format!(
                "__pmBuiltin.command('panel.effect-controls',{args})"
            ))
            .unwrap(),
    );
    let reset = effect(&kernel);
    let keys = reset["p"]["startX"]["kf"].as_array().unwrap();
    assert_eq!(
        keys.len(),
        3,
        "Reset must key at the playhead and retain existing animation"
    );
    for (time, value) in [(0., 10.), (1., 40.), (2., 30.)] {
        assert!(
            keys.iter().any(|k| k["t"] == time && k["v"] == value),
            "missing Reset key {time}:{value}"
        );
    }
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(
        effect(&kernel),
        before,
        "one Undo restores every affected channel"
    );
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Redo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(effect(&kernel), reset);
}

#[test]
fn e07_application_saved_ramp_v1_keeps_legacy_controls_and_reset_defaults() {
    let catalog: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../engine/crates/pm-fx-defs/src/main-effects.json"
    )))
    .unwrap();
    let current = catalog["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "gradient")
        .unwrap();
    let legacy = &current["legacy"][0];
    let p: serde_json::Map<String, Value> = legacy["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["k"].as_str().unwrap().to_owned(),
                json!({"v":p["def"],"kf":[],"expr":null}),
            )
        })
        .collect();
    for version in [None, Some(Value::Null), Some(json!(1))] {
        let mut saved = json!({"id":"saved-ramp","type":"gradient","on":true,"open":true,"p":p});
        saved["p"]["angle"]["v"] = json!(45.);
        if let Some(version) = version {
            saved["version"] = version;
        }
        let (actor, _) = setup_with_saved_effect(Some(saved.clone()));
        let kernel = actor.handle();
        let before = effect(&kernel);
        let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
        finish(panels.mount("layer-effects").unwrap());
        let projection = panels
            .broker
            .entry("layer-effects")
            .unwrap()
            .lock()
            .unwrap()
            .backend
            .clone()
            .lock()
            .unwrap()
            .projection()
            .unwrap();
        let center = projection["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "parameter" && r["parameter"]["label"] == "Center X")
            .expect("saved v1 must retain its own controls");
        assert_eq!(center["parameter"]["readonly"], false);
        let args = json!({"documentId":projection["documentId"],"revision":projection["revision"],"layer":"a","intent":{"type":"reset","id":"saved-ramp"}});
        finish(
            panels
                .realms
                .lock()
                .unwrap()
                .get("layer-effects")
                .unwrap()
                .eval(format!(
                    "__pmBuiltin.command('panel.effect-controls',{args})"
                ))
                .unwrap(),
        );
        let reset = effect(&kernel);
        assert_eq!(reset["p"]["angle"]["v"], 90.);
        assert!(reset["p"].get("startX").is_none());
        assert_eq!(reset.get("version"), saved.get("version"));
        assert_eq!(
            reset["p"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<std::collections::BTreeSet<_>>(),
            saved["p"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<std::collections::BTreeSet<_>>()
        );
        kernel
            .call(
                "synthetic",
                Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
            )
            .wait()
            .unwrap()
            .unwrap();
        assert_eq!(effect(&kernel), before);
    }
}

fn await_effect(
    panels: &mut Builtins,
    kernel: &pm_kernel::KernelHandle,
    ready: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let value = effect(kernel);
        if ready(&value) {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "effect action did not receive its real actor acknowledgement: {:?}",
            panels.take_errors()
        );
        for (_, ticket) in panels.publish() {
            finish(ticket.unwrap());
        }
        thread::sleep(Duration::from_millis(1));
    }
}
fn rendered_project(
    kernel: &pm_kernel::KernelHandle,
    renderer: &mut pm_render::Renderer,
) -> Vec<f32> {
    let published = kernel.published("synthetic").unwrap();
    let document = &published.snapshot.document;
    let _turn = install_main_effects(renderer, document);
    let mut request = pm_render::RenderRequest::sized(
        pm_protocol::common::Size::new(160, 90),
        pm_protocol::color::Precision::Float16,
    );
    request.export = true;
    request.output_kind = pm_render::OutputKind::Final;
    let frame = pm_export::render::render_frame(
        renderer,
        document,
        "synthetic-root",
        pm_protocol::common::Ticks::ZERO,
        &request,
        &pm_export::render::MediaPaths::default(),
        &pm_export::render::Unrestricted,
        1,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    frame.rgba
}
#[test]
fn e07_application_ramp_swap_and_undo_render_the_published_project_offscreen() {
    let (actor, mut backend) = setup();
    let kernel = actor.handle();
    let request = backend
        .model
        .as_mut()
        .unwrap()
        .add_effect_at(Caller::native(), "a", "gradient", 0.)
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let before = effect(&kernel);
    let mut renderer = pm_render::Renderer::headless().unwrap();
    let original = rendered_project(&kernel, &mut renderer);
    let center = (20 * 160 + 40) * 4;
    for channel in 0..3 {
        assert!(
            (f64::from(original[center + channel]) - 20.5 / 40.).abs() <= 1. / 1024.,
            "source-sized ramp output"
        );
    }
    assert_eq!(original[center + 3], 1.);
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("layer-effects").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let button = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("label").and_then(Value::as_str) == Some("Swap Colors"))
        .unwrap();
    finish(
        panels
            .event(
                "layer-effects",
                publication.generation,
                publication.surface.revision(),
                button.id,
                "click",
                EventDetail::default(),
            )
            .unwrap(),
    );
    await_effect(&mut panels, &kernel, |value| {
        value["p"]["startColor"]["v"] == before["p"]["endColor"]["v"]
    });
    let swapped = rendered_project(&kernel, &mut renderer);
    for channel in 0..3 {
        assert!((f64::from(swapped[center + channel]) - (1. - 20.5 / 40.)).abs() <= 1. / 1024.);
    }
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(
        rendered_project(&kernel, &mut renderer),
        original,
        "Undo restores every rendered pixel from the published project"
    );
}

#[test]
fn e04_effect_control_requests_cannot_edit_a_reopened_document_with_reused_ids_and_revision() {
    use pm_panel_inspector::{Binding, PropertyId};
    for action in ["reset", "action", "add", "parameter"] {
        let (actor, mut backend) = setup();
        let kernel = actor.handle();
        let request = backend
            .model
            .as_mut()
            .unwrap()
            .add_effect_at(Caller::native(), "a", "gradient", 0.)
            .unwrap();
        kernel.call("synthetic", request).wait().unwrap().unwrap();
        backend.sync().unwrap();
        let id = effect(&kernel)["id"].as_str().unwrap().to_owned();
        let property = PropertyId::new("a", format!("{id}.startX"));
        let request = backend
            .model
            .as_mut()
            .unwrap()
            .set(
                Caller::native(),
                Binding::Property(property.clone()),
                pm_model::Value::Number(10.),
                0.,
            )
            .unwrap();
        kernel.call("synthetic", request).wait().unwrap().unwrap();
        let projection = backend.projection().unwrap();
        let intent = match action {
            "reset" => json!({"type":"reset","id":id}),
            "action" => json!({"type":"action","id":id,"index":0}),
            "add" => json!({"type":"add","definition":"gradient"}),
            _ => {
                json!({"type":"parameter","id":pm_model::json::to_canonical_string(&property.value()),"action":"diamond"})
            }
        };
        let args = json!({"documentId":projection["documentId"],"revision":projection["revision"],"layer":"a","intent":intent});
        let mut panel =
            pm_panel_inspector::v4_effects::EffectsPanel::new(Default::default(), 0..200);
        let mut plan = panel
            .plan(
                backend.model.as_mut().unwrap(),
                Caller::native(),
                &data::model(&args).unwrap(),
                0.,
            )
            .unwrap();
        let old = kernel.published("synthetic").unwrap();
        let revision = old.revision();
        let replacement = old.snapshot.document.to_value().unwrap();
        assert!(kernel.close("synthetic").wait().unwrap());
        kernel
            .open("synthetic", OpenSource::Untitled(replacement))
            .wait()
            .unwrap()
            .unwrap();
        while kernel.published("synthetic").unwrap().revision() < revision {
            let current_revision = kernel.published("synthetic").unwrap().revision();
            let request = Request::new(
                Caller::native(),
                pm_kernel::Call::Transact {
                    label: Some("Replacement revision".into()),
                    expected_revision: Some(current_revision),
                    body: pm_kernel::WriteBody::Commands(vec![pm_kernel::Command::Comp(
                        pm_kernel::CompCommand::Set {
                            patch: Box::new(pm_kernel::commands::comps::CompPatch {
                                name: Some(format!("Replacement {current_revision}")),
                                ..Default::default()
                            }),
                        },
                    )]),
                },
            );
            kernel.call("synthetic", request).wait().unwrap().unwrap();
        }
        let replacement = kernel.published("synthetic").unwrap();
        assert_ne!(replacement.snapshot.doc_id, old.snapshot.doc_id);
        assert_eq!(
            replacement.revision(),
            revision,
            "the revision guard alone cannot distinguish this lifetime"
        );
        assert_eq!(
            effect(&kernel)["id"],
            id,
            "the authored effect ID is reused"
        );
        assert_eq!(effect(&kernel)["p"]["startX"]["v"], 10.);
        let before = replacement.snapshot.document.to_value();
        let request = plan.request.take().unwrap();
        let mut current = request.clone();
        current.expected_document = Some(replacement.snapshot.doc_id);
        let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
        assert!(
            matches!(ack.reply, pm_kernel::Reply::Throw(_)),
            "stale {action} request must be refused by the actor: {:?}",
            ack.reply
        );
        let after = kernel.published("synthetic").unwrap();
        assert_eq!(after.revision(), revision);
        assert_eq!(
            after.snapshot.document.to_value(),
            before,
            "stale {action} must not mutate replacement artwork"
        );
        let ack = kernel.call("synthetic", current).wait().unwrap().unwrap();
        assert!(
            !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
            "the same {action} operation with current document authority must remain available: {:?}",
            ack.reply
        );
        assert_ne!(
            kernel
                .published("synthetic")
                .unwrap()
                .snapshot
                .document
                .to_value(),
            before
        );
    }
}

/// The inspector's mirror has no fonts, so it cannot measure a text layer.
/// Add and Reset take main's next sizes (layer, then composition) and work.
#[test]
fn ramp_on_a_text_layer_adds_and_resets_without_font_bounds() {
    use pm_panel_inspector::{Binding, PropertyId};
    let (actor, mut backend) = setup_project(|value| {
        value["layers"][0]["type"] = json!("text");
        value["layers"][0]["d"] = json!({"text":"Ramp","size":24,"color":"#FFFFFF"});
    });
    let kernel = actor.handle();
    backend.sync().unwrap();
    let request = backend
        .model
        .as_mut()
        .unwrap()
        .add_effect_at(Caller::native(), "a", "gradient", 0.)
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let created = effect(&kernel);
    assert_eq!(created["p"]["startX"]["v"], 80.);
    assert_eq!(created["p"]["endY"]["v"], 90.);
    let id = created["id"].as_str().unwrap().to_owned();
    backend.sync().unwrap();
    let request = backend
        .model
        .as_mut()
        .unwrap()
        .set(
            Caller::native(),
            Binding::Property(PropertyId::new("a", format!("{id}.startX"))),
            pm_model::Value::Number(10.),
            0.,
        )
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert_eq!(effect(&kernel)["p"]["startX"]["v"], 10.);
    let projection = backend.projection().unwrap();
    let args = json!({"documentId":projection["documentId"],"revision":projection["revision"],"layer":"a","intent":{"type":"reset","id":id}});
    let mut panel = pm_panel_inspector::v4_effects::EffectsPanel::new(Default::default(), 0..200);
    let plan = panel
        .plan(
            backend.model.as_mut().unwrap(),
            Caller::native(),
            &data::model(&args).unwrap(),
            0.,
        )
        .unwrap();
    kernel
        .call("synthetic", plan.request.unwrap())
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(effect(&kernel)["p"]["startX"]["v"], 80.);
}

#[test]
fn a_missing_effect_names_its_parameters_in_words_and_shows_their_values_at_rest() {
    let saved = json!({"id":"lost","type":"ascii-magic-effects.characters","on":true,"open":true,"missing":true,
        "p":{"backgroundColor":{"v":"#112233","kf":[]},"dotGrid":{"v":4,"kf":[]},"amount":{"v":0.5,"kf":[{"t":0,"v":0},{"t":1,"v":1}]}}});
    let (actor, _) = setup_with_saved_effect(Some(saved));
    let kernel = actor.handle();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("layer-effects").unwrap());
    let projection = panels
        .broker
        .entry("layer-effects")
        .unwrap()
        .lock()
        .unwrap()
        .backend
        .clone()
        .lock()
        .unwrap()
        .projection()
        .unwrap();
    let parameter = |label: &str| {
        projection["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "parameter" && r["parameter"]["label"] == label)
            .unwrap_or_else(|| panic!("{label}: {projection:#}"))["parameter"]
            .clone()
    };
    assert_eq!(parameter("Background Color")["value"], "#112233");
    assert_eq!(parameter("Dot Grid")["value"], 4.);
    assert_eq!(parameter("Amount")["value"], "Animated");
    assert_eq!(parameter("Amount")["readonly"], true);
}
