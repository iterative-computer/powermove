use super::*;
use pm_kernel::{Caller, KernelActor, KernelConfig, OpenSource, Request, SelectionCall};
use std::{
    thread,
    time::{Duration, Instant},
};
fn finish(ticket: Ticket<Evaluation>) -> Evaluation {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = ticket.try_take() {
            return result.expect("v4 worker execution");
        }
        assert!(Instant::now() < deadline, "v4 ticket timed out");
        thread::sleep(Duration::from_millis(1));
    }
}
fn actor() -> KernelActor {
    let actor = KernelActor::spawn(KernelConfig::default());
    let project=pm_model::json::parse(r##"{"id":"synthetic","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"a","type":"shape","name":"Headline","from":0,"dur":10,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[]},"position.x":{"v":320,"kf":[]},"position.y":{"v":180,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"##).unwrap();
    actor
        .handle()
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    actor
}
#[test]
fn authored_effect_controls_mount_and_follow_kernel_selection() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("layer-effects").unwrap());
    let first = panels.take_publications().pop().unwrap();
    assert!(
        first
            .surface
            .nodes()
            .values()
            .any(|n| n.text == "Select a layer to see its effects."),
        "{:?}",
        first.surface.nodes()
    );
    actor
        .handle()
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: vec!["a".into()],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    for (_, ticket) in panels.publish() {
        finish(ticket.unwrap());
    }
    let next = panels.take_publications().pop().unwrap();
    assert!(next.surface.nodes().values().any(|n| n.text == "Headline"));
    let prepared = pm_surface::kit::Panel::prepare(
        next.surface,
        &pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default()),
        &Default::default(),
        None,
        false,
    );
    assert!(
        prepared
            .accessibility
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Effect Controls"))
    );
    assert!(panels.take_errors().is_empty());
}
#[test]
fn effects_mounts_in_a_separate_v4_isolate() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("layer-effects").unwrap());
    finish(panels.mount("fxbrowser").unwrap());
    let publications = panels.take_publications();
    assert_eq!(publications.len(), 2);
    assert_ne!(publications[0].generation, publications[1].generation);
    assert!(panels.take_errors().is_empty());
}
#[test]
fn all_nine_authored_entries_mount_and_publish_accessible_native_trees() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    for id in LIVE_PANELS {
        let ticket = panels.mount(id).unwrap();
        finish(ticket);
        if matches!(*id, "settings" | "export") {
            finish(panels.show(id).unwrap());
        }
    }
    let publications = panels.take_publications();
    assert_eq!(publications.len(), 9);
    for p in publications {
        assert!(p.surface.nodes().len() > 1, "{} empty tree", p.panel);
        let kit = pm_surface::kit::Panel::prepare(
            p.surface,
            &pm_design::Tokens::new(pm_design::Scheme::Dark, Default::default()),
            &Default::default(),
            None,
            false,
        );
        assert!(kit.accessibility.nodes.len() > 1, "{} missing AX", p.panel);
        assert!(
            kit.diagnostics.is_empty(),
            "{}: {:?}",
            p.panel,
            kit.diagnostics
        );
    }
    assert!(panels.take_errors().is_empty());
}

#[test]
fn properties_native_binding_writes_kernel_and_retired_selection_rejects_lease() {
    let actor = actor();
    let kernel = actor.handle();
    kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: vec!["a".into()],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("inspector").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let field = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("id").and_then(Value::as_str) == Some("value:opacity:0"))
        .expect("native opacity field");
    let handle = field.props["binding"].as_str().unwrap().to_owned();
    finish(panels.write("inspector", &handle, json!(42.)).unwrap());
    let p = kernel.published("synthetic").unwrap();
    let layer = p
        .snapshot
        .layers(None)
        .into_iter()
        .find(|l| l.id == "a")
        .unwrap();
    let mut inspector = pm_panel_inspector::Inspector::new(p, Default::default());
    assert_eq!(
        inspector
            .binding_state(
                &pm_panel_inspector::Binding::Property(pm_panel_inspector::PropertyId::new(
                    &layer.id, "opacity"
                )),
                0.
            )
            .unwrap()
            .value
            .as_f64(),
        Some(42.)
    );
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), SelectionCall::Clear),
        )
        .wait()
        .unwrap()
        .unwrap();
    for (_, ticket) in panels.publish() {
        finish(ticket.unwrap());
    }
    let stale = panels.write("inspector", &handle, json!(99.)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = stale.try_take() {
            assert!(result.is_err(), "retired selection lease accepted");
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        panels
            .event(
                "inspector",
                publication.generation,
                publication.surface.revision(),
                field.id,
                "change",
                EventDetail {
                    value: Some(json!(99.)),
                    ..Default::default()
                }
            )
            .is_err()
    );
    panels.close();
}

#[test]
fn application_agent_and_settings_mount_without_creating_a_project() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let mut panels = Builtins::application(actor.handle(), || {}).unwrap();
    assert!(panels.mount("inspector").is_err());
    for id in ["agent", "settings"] {
        finish(panels.mount(id).unwrap());
        finish(panels.show(id).unwrap());
    }
    let publications = panels.take_publications();
    assert_eq!(publications.len(), 2);
    let settings = publications.iter().find(|p| p.panel == "settings").unwrap();
    assert!(
        !settings
            .surface
            .nodes()
            .values()
            .any(|n| n.props.get("id").and_then(Value::as_str) == Some("settings.section.project"))
    );
    assert!(actor.handle().published("builtin-application").is_none());
    assert!(panels.take_errors().is_empty());
    panels.close();
}

#[test]
fn application_profile_writes_are_actor_owned_and_document_commands_refuse() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "builtin-application".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", true).unwrap();
    let models::NativeCommand::State(tx) = backend
        .native_command("profile.set", &json!({"key":"themeMode","value":"light"}))
        .unwrap()
    else {
        panic!("state transaction expected")
    };
    actor
        .handle()
        .state_transaction(tx)
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(
        backend
            .native_command("profile.get", &json!({"key":"themeMode"}))
            .map(|v| match v {
                models::NativeCommand::Value(v) => v,
                _ => Value::Null,
            })
            .unwrap()["value"],
        "light"
    );
    assert!(
        backend
            .native_command(
                "panel.inspector",
                &json!({"type":"align","alignment":"left"})
            )
            .is_err()
    );
    assert!(
        backend
            .settings_snapshot()
            .unwrap()
            .get("project")
            .is_none()
    );
}

#[test]
fn properties_publish_applicable_content_and_mixed_type_without_rewriting_data() {
    let actor = actor();
    let kernel = actor.handle();
    kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: vec!["a".into()],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let published = kernel.published("synthetic").unwrap();
    let revision = published.revision();
    let mut inspector = pm_panel_inspector::Inspector::new(published, Default::default());
    let mut presenter = properties::Properties::default();
    let snapshot = presenter.snapshot(&mut inspector, 0., false).unwrap();
    let rows = snapshot["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap());
    let width = rows.into_iter().find(|r| r["id"] == "content.w").unwrap();
    assert_eq!(width["label"], "Width");
    assert_eq!(width["control"]["axes"][0]["value"], 100.);
    assert_eq!(kernel.published("synthetic").unwrap().revision(), revision);
}

#[test]
fn unsupported_agent_submission_keeps_the_draft_and_does_not_start_a_run() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "agent".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "agent", false).unwrap();
    let thread = backend.native.agent.active_id().to_owned();
    backend
        .native
        .agent
        .draft_mut(&thread)
        .unwrap()
        .insert_text("Keep authored 日本🙂", 0, false);
    let result=backend.native_command("panel.agent.dispatch",&json!({"sequence":backend.native.agent_sequence,"target":{"project":"synthetic","thread":thread,"run":null},"action":{"type":"submit"}}));
    assert!(result.is_err());
    assert_eq!(
        backend.native.agent.session(&thread).unwrap().draft.text(),
        "Keep authored 日本🙂"
    );
    assert!(backend.native.agent.session(&thread).unwrap().run.is_none());
}

#[test]
fn settings_project_services_validate_and_publish_only_after_actor_ack() {
    let actor = actor();
    let kernel = actor.handle();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "settings", false).unwrap();
    let publication = backend.settings_snapshot().unwrap();
    let args = json!({"revision":publication["revision"],"action":{"type":"write","id":"project.name","value":"New authored name"}});
    let models::NativeCommand::Kernel(request) =
        backend.native_command("panel.settings", &args).unwrap()
    else {
        panic!("typed kernel request expected")
    };
    assert_eq!(
        backend.settings_snapshot().unwrap()["project"]["name"],
        "Synthetic Film"
    );
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let updated = backend.settings_snapshot().unwrap();
    assert_eq!(updated["project"]["name"], "New authored name");
    assert!(backend.native_command("panel.settings", &args).is_err());
    for value in [json!(0), json!(241), json!("30")] {
        assert!(backend.native_command("panel.settings",&json!({"revision":updated["revision"],"action":{"type":"write","id":"project.fps","value":value}})).is_err());
    }
    let models::NativeCommand::Kernel(request) = backend.native_command("panel.settings", &json!({"revision":updated["revision"],"action":{"type":"write","id":"project.fps","value":29.97}})).unwrap() else { panic!("typed kernel request") };
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let published = kernel.published("synthetic").unwrap();
    let root = published.snapshot.root().unwrap();
    let comp =
        pm_kernel::model::comp_node(root, published.snapshot.open_comp().as_deref()).unwrap();
    assert_eq!(comp.get("fps").unwrap().to_value().as_f64(), Some(29.97));
}

#[test]
fn application_scope_never_exposes_a_real_document_with_the_transport_id() {
    let actor = actor();
    let root = actor
        .handle()
        .published("synthetic")
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value();
    actor
        .handle()
        .open("builtin-application", OpenSource::Untitled(root))
        .wait()
        .unwrap()
        .unwrap();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "builtin-application".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", true).unwrap();
    assert!(backend.published().is_none());
    let revision = backend.settings_snapshot().unwrap()["revision"].clone();
    assert!(backend.native_command("panel.settings", &json!({"revision":revision,"action":{"type":"write","id":"project.name","value":"Must not write"}})).is_err());
    assert!(
        backend
            .settings_snapshot()
            .unwrap()
            .get("project")
            .is_none()
    );
    assert!(
        backend
            .native_command(
                "panel.inspector",
                &json!({"type":"align","alignment":"left"})
            )
            .is_err()
    );
}

#[test]
fn render_presets_use_the_legacy_actor_key_and_archive_same_name_records() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "export".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "export", false).unwrap();
    let options = backend.export_snapshot().unwrap()["options"].clone();
    for _ in 0..2 {
        let models::NativeCommand::State(tx) = backend
            .native_command(
                "export.preset.save",
                &json!({"name":"Master","options":options}),
            )
            .unwrap()
        else {
            panic!("actor write")
        };
        actor
            .handle()
            .state_transaction(tx)
            .wait()
            .unwrap()
            .unwrap();
    }
    let state = actor.handle().state();
    let saved = data::wire(state.settings.get("renderPresets").unwrap());
    assert_eq!(saved.as_array().unwrap().len(), 2);
    assert_eq!(saved[0]["archived"], true);
    assert_ne!(saved[0]["id"], saved[1]["id"]);
    assert_eq!(
        backend.export_snapshot().unwrap()["presets"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!state.settings.contains_key("exportPresets"));
}

#[test]
fn running_export_refuses_replacement_or_reset_without_mutating_its_run() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "export".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "export", false).unwrap();
    backend.export_snapshot().unwrap();
    let run = json!({"phase":"rendering","frame":2,"total":300,"elapsed":1});
    backend.native.export["run"] = run.clone();
    for command in [
        "export.begin",
        "export.retry",
        "export.plan",
        "export.reset",
        "export.closeFailure",
        "export.preset.save",
    ] {
        assert!(backend.native_command(command, &json!({})).is_err());
        assert_eq!(backend.native.export["run"], run);
    }
    assert!(backend.native.platform.is_empty());
}

#[path = "timeline_tests.rs"]
mod timeline;
fn select_layer(kernel: &pm_kernel::KernelHandle, id: &str) {
    let ack = kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: vec![id.into()],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
}

#[test]
fn project_color_depth_uses_real_kernel_receipts_and_one_undo() {
    let actor = actor();
    let kernel = actor.handle();
    let identity = pm_surface::host_api::Identity {
        realm: "settings-depth".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "settings", false).unwrap();
    let publication = backend.settings_snapshot().unwrap();
    let row = publication["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "project.colorDepth")
        .unwrap();
    assert_eq!(row["options"][2], json!({"v":32,"label":"32-bit float"}));
    for value in [json!(12), json!("16"), json!(16.5), json!(null)] {
        assert!(backend.native_command("panel.settings", &json!({"revision":publication["revision"],"action":{"type":"write","id":"project.colorDepth","value":value}})).is_err());
    }
    let before = kernel
        .published("synthetic")
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value();
    let models::NativeCommand::Kernel(request) = backend.native_command("panel.settings", &json!({"revision":publication["revision"],"action":{"type":"write","id":"project.colorDepth","value":32}})).unwrap() else { panic!("actor request") };
    assert_eq!(
        kernel
            .published("synthetic")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .to_value(),
        before
    );
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    assert_eq!(
        kernel
            .published("synthetic")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .get("colorDepth")
            .unwrap()
            .to_value(),
        pm_model::Value::Number(32.)
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
        kernel
            .published("synthetic")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .to_value(),
        before
    );
}

#[test]
fn properties_independent_corners_seed_from_uniform_radius_in_one_actor_edit() {
    let actor = actor();
    let kernel = actor.handle();
    select_layer(&kernel, "a");
    let mut patch = pm_model::Map::new();
    patch.insert("shape", "rect".into());
    patch.insert("radius", 17.5.into());
    patch.insert("independentCorners", false.into());
    kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                pm_kernel::Call::Transact {
                    label: None,
                    expected_revision: None,
                    body: pm_kernel::WriteBody::Commands(vec![
                        pm_kernel::ContentCommand::Set {
                            layer_id: "a".into(),
                            patch,
                            time: 0.,
                        }
                        .into(),
                    ]),
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let mut model = pm_panel_inspector::Inspector::new(
        kernel.published("synthetic").unwrap(),
        Default::default(),
    );
    let mut presenter = properties::Properties::default();
    let publication = presenter.snapshot(&mut model, 0., false).unwrap();
    let row = publication["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap())
        .find(|r| r["id"] == "content.independentCorners")
        .unwrap();
    let request = presenter
        .write(
            &mut model,
            Caller::native(),
            row["control"]["binding"]["handle"].as_str().unwrap(),
            true.into(),
            0.,
        )
        .unwrap();
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    model.sync(kernel.published("synthetic").unwrap());
    let layer = model.layer("a").unwrap();
    for field in ["radiusTL", "radiusTR", "radiusBL", "radiusBR"] {
        assert_eq!(
            layer.get("d").unwrap().get(field).unwrap().as_f64(),
            Some(17.5)
        );
    }
    let publication = presenter.snapshot(&mut model, 0., false).unwrap();
    assert_eq!(
        publication["sections"][0]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["id"]
                .as_str()
                .is_some_and(|s| s.starts_with("content.radius")))
            .count(),
        4
    );
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    model.sync(kernel.published("synthetic").unwrap());
    assert_eq!(
        model
            .layer("a")
            .unwrap()
            .get("d")
            .unwrap()
            .get("independentCorners")
            .unwrap()
            .as_bool(),
        Some(false)
    );
}

fn register_synthetic_effect(kernel: &pm_kernel::KernelHandle) {
    kernel.state_transaction(pm_kernel::state::StateTransaction {
        expected_revision: None,
        ops: vec![pm_kernel::state::StateOp::Register { registry:"effects".into(), entry: pm_kernel::state::Registration {
            token:"synthetic-effect-token".into(), owner:Caller::native().writer,
            id:"synthetic.glow".into(), definition: data::model(&json!({"label":"Synthetic Glow","version":1,"params":[{"k":"amount","label":"Amount","type":"number","def":5,"step":0.25},{"k":"tint","label":"Tint","type":"color","def":"#FF0000"},{"k":"enabled","label":"Enabled","type":"toggle","def":true},{"k":"mode","label":"Mode","type":"select","def":"a","options":[{"v":"a","label":"A"},{"v":"b","label":"B"}]},{"k":"label","label":"Label","type":"text","def":"Authored"}]})).unwrap(),
        }}],
    }).wait().unwrap().unwrap();
}

#[test]
fn properties_header_add_effect_uses_native_menu_then_real_actor_transaction() {
    let actor = actor();
    let kernel = actor.handle();
    register_synthetic_effect(&kernel);
    select_layer(&kernel, "a");
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("inspector").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let button = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("id").and_then(Value::as_str) == Some("inspector:add-effect"))
        .unwrap();
    finish(
        panels
            .event(
                "inspector",
                publication.generation,
                publication.surface.revision(),
                button.id,
                "click",
                EventDetail::default(),
            )
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let request = loop {
        if let Some(request) = panels
            .take_platform_requests()
            .into_iter()
            .find(|r| r.operation == "effectMenu")
        {
            break request;
        }
        assert!(Instant::now() < deadline, "native menu was not enqueued");
        thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(request.panel, "inspector");
    let effect_count = || {
        let p = kernel.published("synthetic").unwrap();
        let root = p.snapshot.root().unwrap();
        let loc = pm_kernel::model::find_layer(root, "a").unwrap();
        pm_kernel::model::layer_at(root, &loc)
            .unwrap()
            .get("fx")
            .unwrap()
            .to_value()
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(effect_count(), 0);
    panels.platform_reply(request.id, Ok(json!("synthetic.glow")));
    loop {
        if effect_count() == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "chosen effect did not reach the kernel: {:?}",
            panels.take_errors()
        );
        // Publish on the worker also polls actual actor receipts.
        for (_, ticket) in panels.publish() {
            finish(ticket.unwrap());
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert!(panels.take_errors().is_empty());
}

#[test]
fn effect_controls_native_binding_and_diamond_use_published_playhead() {
    let actor = actor();
    let kernel = actor.handle();
    register_synthetic_effect(&kernel);
    select_layer(&kernel, "a");
    let mut setup = data::Backend::new(
        kernel.clone(),
        pm_surface::host_api::Identity {
            realm: "effect-setup".into(),
            generation: 1,
            project: "synthetic".into(),
        },
        "layer-effects",
        false,
    )
    .unwrap();
    setup.sync().unwrap();
    let request = setup
        .model
        .as_mut()
        .unwrap()
        .add_effect(Caller::native(), "a", "synthetic.glow")
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    panels.update_facts(json!({"time":2.25,"playing":false,"stage":[640.,360.]}));
    finish(panels.mount("layer-effects").unwrap());
    let projection = panels
        .broker
        .entries
        .lock()
        .unwrap()
        .get("layer-effects")
        .unwrap()
        .backend
        .lock()
        .unwrap()
        .projection()
        .unwrap();
    let parameter = projection["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "parameter")
        .unwrap()["parameter"]
        .clone();
    let handle = parameter["binding"].as_str().unwrap();
    finish(panels.write("layer-effects", handle, json!(7.5)).unwrap());
    let projection = panels
        .broker
        .entries
        .lock()
        .unwrap()
        .get("layer-effects")
        .unwrap()
        .backend
        .lock()
        .unwrap()
        .projection()
        .unwrap();
    let args = json!({"documentId":projection["documentId"],"revision":projection["revision"],"layer":"a","intent":{"type":"parameter","id":parameter["id"],"action":"diamond"}});
    finish(
        panels
            .realms
            .get("layer-effects")
            .unwrap()
            .eval(format!(
                "__pmBuiltin.command('panel.effect-controls',{args})"
            ))
            .unwrap(),
    );
    let p = kernel.published("synthetic").unwrap();
    let mut model = pm_panel_inspector::Inspector::new(p, Default::default());
    let id = pm_panel_inspector::PropertyId::from_value(
        &pm_model::json::parse(parameter["id"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let state = model
        .binding_state(&pm_panel_inspector::Binding::Property(id.clone()), 2.25)
        .unwrap();
    assert_eq!(state.value.as_f64(), Some(7.5));
    assert!(state.key_at_playhead);
    assert!(
        !model
            .binding_state(&pm_panel_inspector::Binding::Property(id), 0.)
            .unwrap()
            .key_at_playhead
    );
    // A selection change invalidates the registered native field lease.
    kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: vec![],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let ticket = panels.write("layer-effects", handle, json!(99.)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = ticket.try_take() {
            assert!(result.is_err());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn properties_row_menu_separators_do_not_replace_the_following_actions() {
    let (operation, options) = inspector_platform("inspector", &json!({"operation":"inspector","input":{"type":"row-menu","items":[
        {"action":"add-key","label":"Add Keyframe at Playhead","enabled":true},
        {"action":"edit-expression","label":"Add Expression…","enabled":true,"separatorBefore":true},
        {"action":"reset","label":"Reset","enabled":false,"separatorBefore":true}
    ]}})).map(|(op, input)| (op.to_owned(), input)).unwrap();
    assert_eq!(operation, "contextMenu");
    assert_eq!(options.as_array().unwrap().len(), 5);
    assert!(options[1].is_null());
    assert_eq!(options[2]["id"], "edit-expression");
    assert!(options[3].is_null());
    assert_eq!(options[4]["id"], "reset");
    assert_eq!(options[4]["disabled"], true);
}

#[test]
fn viewer_repeated_publications_keep_usable_native_tickets_until_model_changes() {
    let actor = actor();
    let kernel = actor.handle();
    let identity = pm_surface::host_api::Identity {
        realm: "viewer-tickets".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "viewer", false).unwrap();
    let first = backend.viewer_snapshot().unwrap();
    for _ in 0..10 {
        assert_eq!(backend.viewer_snapshot().unwrap(), first);
    }
    let args = json!({"ticket":first["model"]["ticket"],"element":"viewer.zoom","argument":2});
    let models::NativeCommand::State(tx) = backend
        .native_command("viewer.chrome.drive", &args)
        .unwrap()
    else {
        panic!("actor state write")
    };
    kernel.state_transaction(tx).wait().unwrap().unwrap();
    let updated = backend.viewer_snapshot().unwrap();
    assert_ne!(updated["model"]["ticket"], first["model"]["ticket"]);
    assert_eq!(updated["model"]["shownZoom"], 2.);
    assert!(
        backend
            .native_command("viewer.chrome.drive", &args)
            .is_err()
    );
    backend.native.facts["stage"] = json!([800., 600.]);
    assert_ne!(
        backend.viewer_snapshot().unwrap()["model"]["ticket"],
        updated["model"]["ticket"]
    );
}

#[test]
fn media_completion_reaches_actor_without_fabricating_readiness_and_relink_undo_keeps_data() {
    let actor = actor();
    let kernel = actor.handle();
    let panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    let catalog =
        pm_panel_media::Catalog::published(&kernel.published("synthetic").unwrap()).unwrap();
    let stored = pm_panel_media::StoredAsset {
        id: "imported-image".into(),
        original_version: "archived-original-v1".into(),
        metadata: data::model(
            &json!({"name":"Synthetic.png","kind":"image","w":80,"h":40,"future":{"keep":true}}),
        )
        .unwrap(),
    };
    let ack = panels
        .admit_media(
            &catalog.stamp,
            vec![stored.clone()],
            None,
            None,
            Caller::native(),
        )
        .unwrap()
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let imported = kernel.published("synthetic").unwrap();
    let imported_root = imported.snapshot.root().unwrap().to_value();
    let catalog = pm_panel_media::Catalog::published(&imported).unwrap();
    use pm_panel_media::MediaRuntime;
    let unknown = pm_panel_media::readiness::ObservedRuntime::default();
    assert_eq!(
        unknown.state(&catalog.stamp, "imported-image").availability,
        pm_panel_media::Availability::Unknown
    );
    let replacement = pm_panel_media::StoredAsset {
        id: "worker-temp-id".into(),
        original_version: "archived-original-v2".into(),
        metadata: data::model(&json!({"name":"Relinked.png","kind":"image","w":100,"h":50}))
            .unwrap(),
    };
    let ack = panels
        .admit_media(
            &catalog.stamp,
            vec![replacement],
            None,
            Some("imported-image".into()),
            Caller::native(),
        )
        .unwrap()
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let relinked = kernel.published("synthetic").unwrap();
    let meta = relinked
        .snapshot
        .root()
        .unwrap()
        .get("assets")
        .unwrap()
        .get("imported-image")
        .unwrap()
        .to_value();
    assert_eq!(
        meta.get("originalVersion")
            .and_then(pm_model::Value::as_str),
        Some("archived-original-v2")
    );
    assert_eq!(
        meta.get("future")
            .unwrap()
            .get("keep")
            .and_then(pm_model::Value::as_bool),
        Some(true)
    );
    // Stale and foreign completions never reach the actor; stored versions remain the service's archives.
    assert!(
        panels
            .admit_media(
                &catalog.stamp,
                vec![stored.clone()],
                None,
                None,
                Caller::native()
            )
            .is_err()
    );
    let mut foreign = catalog.stamp.clone();
    foreign.project = "other".into();
    assert!(
        panels
            .admit_media(&foreign, vec![stored], None, None, Caller::native())
            .is_err()
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
        kernel
            .published("synthetic")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .to_value(),
        imported_root
    );
}

#[test]
fn effect_controls_edits_every_supported_scalar_type_through_registered_native_fields() {
    let actor = actor();
    let kernel = actor.handle();
    register_synthetic_effect(&kernel);
    select_layer(&kernel, "a");
    let mut setup = data::Backend::new(
        kernel.clone(),
        pm_surface::host_api::Identity {
            realm: "effect-scalars".into(),
            generation: 1,
            project: "synthetic".into(),
        },
        "layer-effects",
        false,
    )
    .unwrap();
    setup.sync().unwrap();
    let request = setup
        .model
        .as_mut()
        .unwrap()
        .add_effect(Caller::native(), "a", "synthetic.glow")
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("layer-effects").unwrap());
    for (label, value) in [
        ("Amount", json!(8.25)),
        ("Tint", json!("#123456")),
        ("Enabled", json!(false)),
        ("Mode", json!("b")),
        ("Label", json!("日本🙂\nSecond line")),
    ] {
        let projection = panels
            .broker
            .entries
            .lock()
            .unwrap()
            .get("layer-effects")
            .unwrap()
            .backend
            .lock()
            .unwrap()
            .projection()
            .unwrap();
        let parameter = projection["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "parameter" && r["parameter"]["label"] == label)
            .unwrap()["parameter"]
            .clone();
        assert_eq!(parameter["readonly"], false);
        finish(
            panels
                .write(
                    "layer-effects",
                    parameter["binding"].as_str().unwrap(),
                    value.clone(),
                )
                .unwrap(),
        );
        let after = panels
            .broker
            .entries
            .lock()
            .unwrap()
            .get("layer-effects")
            .unwrap()
            .backend
            .lock()
            .unwrap()
            .projection()
            .unwrap();
        let actual = after["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "parameter" && r["parameter"]["label"] == label)
            .unwrap()["parameter"]["value"]
            .clone();
        assert_eq!(actual, value);
    }
    assert!(panels.take_errors().is_empty());
}

#[test]
fn collapsed_properties_dispose_native_binding_capabilities() {
    let actor = actor();
    let kernel = actor.handle();
    select_layer(&kernel, "a");
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("inspector").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let handle = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("id").and_then(Value::as_str) == Some("value:opacity:0"))
        .unwrap()
        .props["binding"]
        .as_str()
        .unwrap()
        .to_owned();
    finish(panels.realms.get("inspector").unwrap().eval("__pmBuiltin.command('panel.inspector',{type:'collapse',section:'transform',collapsed:true})").unwrap());
    let before = kernel.published("synthetic").unwrap().revision();
    let ticket = panels.write("inspector", &handle, json!(77.)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = ticket.try_take() {
            assert!(result.is_err());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(kernel.published("synthetic").unwrap().revision(), before);
}
