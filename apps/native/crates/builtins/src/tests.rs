use super::*;
use pm_kernel::{Caller, KernelActor, KernelConfig, OpenSource, Request, SelectionCall};
use std::{
    thread,
    time::{Duration, Instant},
};
/// Two compiler slots serve the whole test process. Render tests take turns
/// (hold the guard until the frame is drawn), and a slot another test still
/// holds is transient, as the engine service treats it.
fn install_main_effects(
    renderer: &mut pm_render::Renderer,
    document: &pm_model::Document,
) -> std::sync::MutexGuard<'static, ()> {
    static RENDER: Mutex<()> = Mutex::new(());
    let turn = RENDER.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = Instant::now() + Duration::from_secs(30);
    while let Err(e) = renderer.set_main_effects_for_document(document) {
        assert!(
            e.contains("compiler-busy") && Instant::now() < deadline,
            "{e}"
        );
        thread::sleep(Duration::from_millis(20));
    }
    turn
}
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
fn a_finder_drop_on_the_project_panel_imports_into_that_target_without_a_layer() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("assets").unwrap());
    let mut publication = None;
    let deadline = Instant::now() + Duration::from_secs(20);
    while publication.is_none() {
        assert!(Instant::now() < deadline, "the panel publishes a tree");
        publication = panels
            .take_publications()
            .into_iter()
            .rfind(|p| p.panel == "assets");
        thread::sleep(Duration::from_millis(1));
    }
    let publication = publication.unwrap();
    let target = publication
        .surface
        .nodes()
        .values()
        .find(|node| node.listeners.contains_key("drop"))
        .expect("the panel declares a drop target")
        .id;
    let files = json!(["/synthetic/clip.mov"]);
    let ticket = panels
        .event(
            "assets",
            publication.generation,
            publication.surface.revision(),
            target,
            "drop",
            EventDetail {
                alt_key: Some(false),
                drag: Some(pm_surface::events::DragData {
                    types: vec!["public.file-url".into()],
                    data: std::collections::BTreeMap::from([("public.file-url".to_owned(), files)]),
                    effect: pm_surface::events::DragEffect::Copy,
                }),
                ..Default::default()
            },
        )
        .unwrap();
    finish(ticket);
    let deadline = Instant::now() + Duration::from_secs(20);
    let request = loop {
        if let Some(request) = panels
            .take_platform_requests()
            .into_iter()
            .find(|r| r.operation == "project.effect")
        {
            break request;
        }
        assert!(Instant::now() < deadline, "{:?}", panels.take_errors());
        thread::sleep(Duration::from_millis(1));
    };
    let effect: pm_panel_media::HostRequest = serde_json::from_value(request.input).unwrap();
    assert_eq!(
        effect.effect,
        pm_panel_media::Effect::Import {
            files: vec!["/synthetic/clip.mov".into()],
            folder: None,
            as_folder: false,
        }
    );
    panels.close();
}

#[test]
fn application_agent_and_settings_mount_without_creating_a_project() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let mut panels = Builtins::application(actor.handle(), || {}, None).unwrap();
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
fn a_chosen_font_and_a_chosen_colour_reach_the_layer_through_their_rows() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let project = pm_model::json::parse(r##"{"id":"synthetic","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"a","type":"text","name":"Title","from":0,"dur":10,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[]}},"d":{"text":"Title","font":"Helvetica","color":"#112233"},"fx":[]}]}"##).unwrap();
    kernel
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
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
    let mut model = pm_panel_inspector::Inspector::new(
        kernel.published("synthetic").unwrap(),
        Default::default(),
    );
    let mut presenter = properties::Properties::default();
    let publication = presenter.snapshot(&mut model, 0., false).unwrap();
    let rows: Vec<_> = publication["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap())
        .collect();
    let font = rows
        .iter()
        .find(|r| r["control"]["kind"] == "font")
        .expect("font row");
    assert_ne!(font["control"]["disabled"], true);
    let request = presenter
        .intent(
            &mut model,
            Caller::native(),
            &json!({"type":"set-value","row":font["id"],"value":"Avenir"}),
            0.,
        )
        .unwrap()
        .unwrap();
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let colour = rows
        .iter()
        .find(|r| r["control"]["kind"] == "colour")
        .expect("colour row");
    model.sync(kernel.published("synthetic").unwrap());
    let request = presenter
        .write(
            &mut model,
            Caller::native(),
            colour["control"]["binding"]["handle"].as_str().unwrap(),
            "#ABCDEF".into(),
            0.,
        )
        .unwrap();
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    model.sync(kernel.published("synthetic").unwrap());
    let written = model.layer("a").unwrap().to_owned();
    let text = pm_model::json::to_canonical_string(&written);
    assert!(text.contains("Avenir"), "{text}");
    assert!(text.contains("#ABCDEF"), "{text}");
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
    let agent = backend.native.agent.clone().unwrap();
    let thread = agent.lock().unwrap().panel.active_id().to_owned();
    agent
        .lock()
        .unwrap()
        .panel
        .draft_mut(&thread)
        .unwrap()
        .insert_text("Keep authored 日本🙂", 0, false);
    let result=backend.native_command("panel.agent.dispatch",&json!({"sequence":backend.native.agent_link.sequence(),"target":{"project":"synthetic","thread":thread,"run":null},"action":{"type":"submit"}}));
    assert!(result.is_err());
    let state = agent.lock().unwrap();
    assert_eq!(
        state.panel.session(&thread).unwrap().draft.text(),
        "Keep authored 日本🙂"
    );
    assert!(state.panel.session(&thread).unwrap().run.is_none());
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
        "export.without",
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
fn the_hdr_setting_reaches_the_kernel_and_unlocks_its_peak() {
    let actor = actor();
    let kernel = actor.handle();
    let identity = pm_surface::host_api::Identity {
        realm: "settings-hdr".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "settings", false).unwrap();
    let row = |publication: &serde_json::Value, id: &str| {
        publication["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()
            .clone()
    };
    let publication = backend.settings_snapshot().unwrap();
    assert_eq!(row(&publication, "project.hdr")["value"], json!(false));
    assert_eq!(
        row(&publication, "project.masteringPeak")["disabled"],
        json!(true)
    );
    let models::NativeCommand::Kernel(request) = backend.native_command("panel.settings", &json!({"revision":publication["revision"],"action":{"type":"write","id":"project.hdr","value":true}})).unwrap() else { panic!("actor request") };
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let root = kernel.published("synthetic").unwrap();
    let root = root.snapshot.root().unwrap();
    assert_eq!(
        pm_render::composite::project_color(root).unwrap().mode,
        pm_protocol::color::ColorMode::Hdr
    );
    let publication = backend.settings_snapshot().unwrap();
    assert_eq!(row(&publication, "project.hdr")["value"], json!(true));
    assert_eq!(
        row(&publication, "project.masteringPeak")["disabled"],
        json!(false)
    );
    assert_eq!(row(&publication, "project.colorDepth")["value"], json!(32));
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
        .add_effect_at(Caller::native(), "a", "synthetic.glow", 0.)
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    panels.update_facts(json!({"time":2.25,"playing":false,"stage":[640.,360.]}));
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
    let args = json!({"documentId":projection["documentId"],"revision":projection["revision"],"layer":"a","intent":{"type":"parameter","id":parameter["id"],"action":"diamond"}});
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
    // Auto's measured playing resolution reaches the Preview Resolution menu.
    assert_eq!(
        backend.viewer_snapshot().unwrap()["model"]["quality"],
        "auto"
    );
    backend.native.facts["effectiveQuality"] = json!(0.5);
    assert_eq!(
        backend.viewer_snapshot().unwrap()["model"]["effectiveQuality"],
        0.5
    );
}

#[test]
fn media_completion_reaches_actor_with_actual_originals_and_relink_undo_keeps_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.path().join("journals")),
        ..Default::default()
    });
    let kernel = actor.handle();
    let profile = dir.path().join("profile");
    kernel
        .shell(pm_kernel::session::ShellCall::AttachLibrary {
            profile: profile.clone(),
            format: pm_kernel::persistence::DiskFormat::V1_2,
            electron_profile: None,
        })
        .wait()
        .unwrap()
        .unwrap();
    let project=pm_model::json::parse(r#"{"id":"synthetic","name":"Synthetic","w":640,"h":360,"fps":30,"dur":5,"layers":[],"comps":{},"assets":{}}"#).unwrap();
    kernel
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let archive = pm_kernel::media_archive::Archive::open(&profile).unwrap();
    let original = |name: &str, bytes: &[u8]| {
        let source = dir.path().join(name);
        std::fs::write(&source, bytes).unwrap();
        archive
            .retain_path(
                &source,
                "application/octet-stream",
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap()
    };
    let panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    let catalog =
        pm_panel_media::Catalog::published(&kernel.published("synthetic").unwrap()).unwrap();
    let first = original("first.bin", b"first complete original");
    let stored_version = first.version().to_owned();
    let stored=RetainedAsset{id:"imported-image".into(),metadata:data::model(&json!({"name":"Synthetic.bin","kind":"image","mimeType":"application/octet-stream","w":80,"h":40,"future":{"keep":true}})).unwrap(),original:first,previous:None};
    let ack = panels
        .admit_media(
            &catalog.stamp,
            vec![stored],
            None,
            None,
            vec![],
            Caller::native(),
        )
        .unwrap()
        .wait()
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)), "{ack:?}");
    assert!(ack.notice.is_none(), "{ack:?}");
    let imported = kernel.published("synthetic").unwrap();
    let catalog = pm_panel_media::Catalog::published(&imported).unwrap();
    use pm_panel_media::MediaRuntime;
    let unknown = pm_panel_media::readiness::ObservedRuntime::default();
    assert_eq!(
        unknown.state(&catalog.stamp, "imported-image").availability,
        pm_panel_media::Availability::Unknown
    );
    let prior = archive
        .retain_entry(imported.media.get("imported-image").unwrap())
        .unwrap();
    let replacement = original("replacement.bin", b"replacement complete original");
    let replacement_version = replacement.version().to_owned();
    let ack=panels.admit_media(&catalog.stamp,vec![RetainedAsset{id:"worker-temp-id".into(),metadata:data::model(&json!({"name":"Relinked.bin","kind":"image","mimeType":"application/octet-stream","w":100,"h":50})).unwrap(),original:replacement,previous:Some(prior)}],None,Some("imported-image".into()),vec![],Caller::native()).unwrap().wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)), "{ack:?}");
    assert!(ack.notice.is_none(), "{ack:?}");
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
        Some(replacement_version.as_str())
    );
    assert_eq!(
        meta.get("future").unwrap().get("keep"),
        Some(&pm_model::Value::Bool(true))
    );
    let copied = dir.path().join("new-copy");
    relinked
        .media
        .get("imported-image")
        .unwrap()
        .copy_to(&copied)
        .unwrap();
    assert_eq!(
        std::fs::read(copied).unwrap(),
        b"replacement complete original"
    );
    let stale = original("stale.bin", b"stale original");
    let retained_path = stale.path();
    assert!(
        panels
            .admit_media(
                &catalog.stamp,
                vec![RetainedAsset {
                    id: "stale".into(),
                    metadata: data::model(&json!({"name":"Stale.bin","kind":"image"})).unwrap(),
                    original: stale,
                    previous: None
                }],
                None,
                None,
                vec![],
                Caller::native()
            )
            .is_err()
    );
    assert_eq!(std::fs::read(retained_path).unwrap(), b"stale original");
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    let undone = kernel.published("synthetic").unwrap();
    let copied = dir.path().join("old-copy");
    undone
        .media
        .get("imported-image")
        .unwrap()
        .copy_to(&copied)
        .unwrap();
    assert_eq!(std::fs::read(copied).unwrap(), b"first complete original");
    assert_eq!(
        undone
            .snapshot
            .root()
            .unwrap()
            .get("assets")
            .unwrap()
            .get("imported-image")
            .unwrap()
            .get("originalVersion")
            .and_then(|n| match n.kind() {
                pm_model::NodeKind::String(s) => s.as_str(),
                _ => None,
            }),
        Some(stored_version.as_str())
    );
    assert!(
        undone
            .snapshot
            .root()
            .unwrap()
            .get("nativeMediaVersions")
            .is_some()
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
        .add_effect_at(Caller::native(), "a", "synthetic.glow", 0.)
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
    finish(panels.realms.lock().unwrap().get("inspector").unwrap().eval("__pmBuiltin.command('panel.inspector',{type:'collapse',section:'transform',collapsed:true})").unwrap());
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

#[test]
fn memory_settings_follow_durable_metadata_without_history_or_edited_changes() {
    let dir = tempfile::tempdir().unwrap();
    let actor = KernelActor::spawn(KernelConfig {
        journal_root: Some(dir.path().join("journals")),
        ..Default::default()
    });
    let kernel = actor.handle();
    let project=pm_model::json::parse(r#"{"id":"P","name":"Synthetic","w":320,"h":180,"fps":30,"dur":2,"layers":[],"assets":{},"comps":{},"future":"\ud800"}"#).unwrap();
    kernel
        .open("P", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let identity = pm_surface::host_api::Identity {
        realm: "settings-memory".into(),
        generation: 1,
        project: "P".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "settings", false).unwrap();
    let initial = backend.settings_snapshot().unwrap();
    let before = kernel.published("P").unwrap();
    let ack = kernel
        .call(
            "P",
            Request::new(
                Caller::realm(7, 1, pm_kernel::Origin::Mcp),
                pm_kernel::MemoryCall::Mutate {
                    expected: None,
                    operation: pm_model::memory::Operation::Add {
                        kind: "decision".into(),
                        text: "Synthetic intent".into(),
                        why: Some("Synthetic reason".into()),
                        resolved: true,
                    },
                    now: 1000.,
                    id: "note".into(),
                },
            )
            .with_document(before.snapshot.doc_id),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert!(
        matches!(ack.reply, pm_kernel::Reply::Memory { .. }),
        "{ack:?}"
    );
    let noted = backend.settings_snapshot().unwrap();
    assert!(noted["revision"].as_u64().unwrap() > initial["revision"].as_u64().unwrap());
    assert_eq!(noted["memory"]["entries"][0]["text"], "Synthetic intent");
    assert_eq!(noted["memory"]["entries"][0]["why"], "Synthetic reason");
    assert_eq!(noted["memory"]["entries"][0]["resolved"], true);
    assert!(backend.native_command("panel.settings", &json!({"revision":initial["revision"],"action":{"type":"write","id":"memory.enabled","value":false}})).is_err());
    let models::NativeCommand::Kernel(request) = backend.native_command("panel.settings", &json!({"revision":noted["revision"],"action":{"type":"write","id":"memory.enabled","value":false}})).unwrap() else { panic!("metadata request") };
    assert_eq!(request.expected_document, Some(before.snapshot.doc_id));
    let ack = kernel.call("P", request).wait().unwrap().unwrap();
    assert!(
        matches!(ack.reply, pm_kernel::Reply::Memory { .. }),
        "{ack:?}"
    );
    assert!(ack.notice.is_none(), "{ack:?}");
    let disabled = backend.settings_snapshot().unwrap();
    assert_eq!(disabled["memory"]["enabled"], false);
    assert_eq!(disabled["memory"]["entries"], noted["memory"]["entries"]);
    let after = kernel.published("P").unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(after.history.entries, before.history.entries);
    assert_eq!(after.dirty, before.dirty);
    let models::NativeCommand::Kernel(request) = backend.native_command("panel.settings", &json!({"revision":disabled["revision"],"action":{"type":"write","id":"memory.enabled","value":true}})).unwrap() else { panic!("metadata request") };
    let ack = kernel.call("P", request).wait().unwrap().unwrap();
    assert!(
        matches!(ack.reply, pm_kernel::Reply::Memory { .. }),
        "{ack:?}"
    );
    assert!(
        kernel
            .published("P")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .get("memoryEnabled")
            .is_none()
    );
    let mut panels = Builtins::new(kernel.clone(), "P", || {}).unwrap();
    finish(panels.mount("settings").unwrap());
    finish(panels.show("settings").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    assert!(
        publication
            .surface
            .nodes()
            .values()
            .any(|n| n.text == "Synthetic intent")
    );
    assert!(
        publication
            .surface
            .nodes()
            .values()
            .any(|n| n.text == "Synthetic reason")
    );
    assert!(
        publication
            .surface
            .nodes()
            .values()
            .any(|n| n.text.contains("Resolved"))
    );
    assert!(panels.take_errors().is_empty());
    actor.shutdown();
}

#[test]
fn a_third_party_surface_extension_mounts_its_own_realm_and_publishes_through_the_native_path() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": "import { jsx } from 'powermove/jsx-runtime';export const view = () => jsx('text', {children:'Synthetic surface'});"
        },
        "edges": {
            "powermove:bundle": {"./index.tsx":"index.tsx"},
            "index.tsx": {"powermove/jsx-runtime":"runtime:powermove/jsx-runtime"}
        }
    });
    finish(
        panels
            .mount_extension("synthetic-surface", &graph, None)
            .unwrap(),
    );
    let publication = panels
        .take_publications()
        .into_iter()
        .find(|p| p.panel == "synthetic-surface")
        .expect("a surface publication");
    assert_eq!(publication.panel, "synthetic-surface");
    // A view is a component: its hooks have somewhere to live.
    let hooks = json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": "import { jsx } from 'powermove/jsx-runtime';import { useState } from 'powermove/hooks';export const view = () => jsx('text', {children:'Hooked '+useState(7)[0]});"
        },
        "edges": {
            "powermove:bundle": {"./index.tsx":"index.tsx"},
            "index.tsx": {
                "powermove/jsx-runtime":"runtime:powermove/jsx-runtime",
                "powermove/hooks":"runtime:powermove/hooks"
            }
        }
    });
    finish(
        panels
            .mount_extension("synthetic-hooks", &hooks, None)
            .unwrap(),
    );
    let hooked = panels
        .take_publications()
        .into_iter()
        .find(|p| p.panel == "synthetic-hooks")
        .expect("a hooked publication");
    assert!(
        hooked
            .surface
            .nodes()
            .values()
            .any(|n| n.text == "Hooked 7")
    );
    // Unmounting frees the id for a rebuilt graph.
    assert!(
        panels
            .mount_extension("synthetic-hooks", &hooks, None)
            .is_err()
    );
    panels.unmount_extension("synthetic-hooks");
    finish(
        panels
            .mount_extension("synthetic-hooks", &hooks, None)
            .unwrap(),
    );
    assert!(panels.mount_extension("timeline", &graph, None).is_err());
    let errors = panels.take_errors();
    assert!(errors.is_empty(), "{errors:?}");
}

/// Only built-in panels' bounded queue plumbing skips the operation quota;
/// a third-party surface sharing the broker stays metered for every operation.
#[test]
fn only_built_in_realms_skip_the_quota_for_queue_operations() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let realm = |id: &str| Generation {
        realm: format!("scope:{:?}", RealmKind::V4Extension(id.into())),
        number: 1,
    };
    let broker: &dyn EngineProxy = &*panels.broker;
    for op in [
        "host.enqueue",
        "host.poll",
        "platform.enqueue",
        "platform.poll",
    ] {
        assert!(!broker.metered(&realm("inspector"), op), "{op}");
        assert!(broker.metered(&realm("synthetic-surface"), op), "{op}");
    }
    assert!(broker.metered(&realm("inspector"), "read"));
    actor.shutdown();
}

fn text_of(panels: &Builtins, panel: &str) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(p) = panels
            .take_publications()
            .into_iter()
            .find(|p| p.panel == panel)
        {
            return p.surface.nodes().values().map(|n| n.text.clone()).collect();
        }
        assert!(Instant::now() < deadline, "no publication for {panel}");
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn a_reloaded_extension_gets_its_hot_data_and_a_fork_takes_a_built_in_panel_and_gives_it_back() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": "import { jsx } from 'powermove/jsx-runtime'; let data = {};
                export function activate({ hot }) { data = hot.data; hot.dispose((d) => { d.count = (d.count ?? 0) + 1; }); }
                export const view = () => jsx('text', { children: 'Count ' + (data.count ?? 0) });"
        },
        "edges": {
            "powermove:bundle": {"./index.tsx":"index.tsx"},
            "index.tsx": {"powermove/jsx-runtime":"runtime:powermove/jsx-runtime"}
        }
    });
    finish(
        panels
            .mount_extension("synthetic-hot", &graph, None)
            .unwrap(),
    );
    assert!(text_of(&panels, "synthetic-hot").contains(&"Count 0".to_owned()));
    let data = finish(panels.hot("synthetic-hot").unwrap()).value;
    assert_eq!(
        serde_json::from_str::<Value>(&data).unwrap(),
        json!({"count":1})
    );
    panels.unmount_extension("synthetic-hot");
    finish(
        panels
            .mount_extension("synthetic-hot", &graph, Some(&data))
            .unwrap(),
    );
    assert!(text_of(&panels, "synthetic-hot").contains(&"Count 1".to_owned()));

    // A fork draws the built-in's panel id with the built-in's routes.
    finish(panels.mount_builtin("viewer").unwrap());
    text_of(&panels, "viewer");
    finish(panels.replace("viewer", "viewer-fork", &graph).unwrap());
    assert!(text_of(&panels, "viewer").contains(&"Count 0".to_owned()));
    finish(panels.restore("viewer").unwrap());
    assert!(
        !text_of(&panels, "viewer")
            .iter()
            .any(|t| t.starts_with("Count"))
    );
    assert!(panels.replace("not-a-panel", "fork", &graph).is_err());
    panels.close();
    actor.shutdown();
}

#[test]
fn mounted_native_worker_cannot_adopt_a_reopened_document_reusing_project_and_layer_ids() {
    let actor = actor();
    let handle = actor.handle();
    let mut old = Builtins::new(handle.clone(), "synthetic", || {}).unwrap();
    finish(old.mount("timeline").unwrap());
    let before = handle.published("synthetic").unwrap();
    let source = before.snapshot.document.to_value().unwrap();
    assert!(handle.close("synthetic").wait().unwrap());
    handle
        .open("synthetic", OpenSource::Untitled(source))
        .wait()
        .unwrap()
        .unwrap();
    let reopened = handle.published("synthetic").unwrap();
    assert_ne!(before.snapshot.doc_id, reopened.snapshot.doc_id);
    let intent = serde_json::json!({"type":"switch", "id":pm_panel_timeline::v4::row_id(&pm_panel_timeline::RowId::Layer("a".into())), "name":"visibility"});
    let ticket = old.timeline_action(intent.clone()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let rejected = loop {
        if let Some(result) = ticket.try_take() {
            break result;
        }
        assert!(
            Instant::now() < deadline,
            "retired native worker must finish"
        );
        thread::sleep(Duration::from_millis(1));
    };
    assert!(
        rejected.is_err(),
        "Old rendered actions must not be rebound: {rejected:?}"
    );
    let after = handle.published("synthetic").unwrap();
    assert_eq!(
        after.snapshot.document.to_value(),
        reopened.snapshot.document.to_value()
    );
    assert_eq!(after.history.entries, reopened.history.entries);
    assert_eq!(after.dirty, reopened.dirty);
    let mut current = Builtins::new(handle.clone(), "synthetic", || {}).unwrap();
    finish(current.mount("timeline").unwrap());
    finish(current.timeline_action(intent).unwrap());
    let changed = handle.published("synthetic").unwrap();
    let open = changed.snapshot.open_comp();
    assert_eq!(
        pm_kernel::model::comp_node(changed.snapshot.root().unwrap(), open.as_deref())
            .unwrap()
            .get("layers")
            .unwrap()
            .at(0)
            .unwrap()
            .get("on")
            .map(pm_model::Node::to_value),
        Some(pm_model::Value::Bool(false))
    );
    assert_eq!(changed.history.entries, reopened.history.entries + 1);
    old.close();
    current.close();
    actor.shutdown();
}

mod effect_delta;
mod effect_registry;
mod text_animators;

mod precomp_delta;

#[test]
fn project_rows_follow_a_media_observation_that_lands_without_an_edit() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let project=pm_model::json::parse(r##"{"id":"synthetic","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{"clip":{"id":"clip","name":"clip.mov","kind":"video","w":640,"h":360,"dur":5,"size":10,"sourcePath":"/synthetic/offline/clip.mov"}},"comps":{},"layers":[]}"##).unwrap();
    actor
        .handle()
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("assets").unwrap());
    let shows = |panels: &Builtins, text: &str| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            for (_, ticket) in panels.publish() {
                finish(ticket.unwrap());
            }
            if panels
                .take_publications()
                .into_iter()
                .filter(|p| p.panel == "assets")
                .any(|p| format!("{:?}", p.surface.nodes()).contains(text))
            {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        false
    };
    assert!(shows(&panels, "clip.mov"), "the asset row publishes");
    // The worker's probe lands at the same revision: nothing in the document moved.
    let published = actor.handle().published("synthetic").unwrap();
    let stamp = pm_panel_media::Stamp {
        project: "synthetic".into(),
        document: published.snapshot.doc_id,
        revision: published.revision(),
    };
    let offline = pm_panel_media::RuntimeState {
        availability: pm_panel_media::Availability::Offline,
        ..Default::default()
    };
    panels.update_media(
        Default::default(),
        pm_panel_media::readiness::ObservedRuntime::new(
            stamp,
            std::collections::BTreeMap::from([("clip".to_owned(), offline)]),
        ),
    );
    assert!(
        shows(&panels, "Missing") && !shows(&panels, "Loading"),
        "the row shows the observation instead of staying Loading"
    );
    panels.close();
}

/// Projects whose layers sit at the root (no `compId`) are the composition
/// the shipped app calls `root`: the timeline shows it and names its tab.
#[test]
fn root_level_project_has_a_timeline_composition_and_tab() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let project = pm_model::json::parse(r##"{"id":"root-film","name":"Root Film","w":32,"h":18,"fps":29.97,"dur":2,"assets":{},"comps":{},"layers":[{"id":"a","type":"shape","name":"Box","from":0,"dur":2,"on":true,"p":{"opacity":{"v":100,"kf":[]}},"d":{"w":10,"h":10},"fx":[]}]}"##).unwrap();
    actor
        .handle()
        .open("root-film", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let identity = pm_surface::host_api::Identity {
        realm: "root-timeline".into(),
        generation: 1,
        project: "root-film".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "timeline", false).unwrap();
    let frame = backend
        .timeline_frame(&json!({"first":0,"visible":20}))
        .unwrap();
    assert_eq!(frame["hasComposition"], true);
    assert_eq!(frame["tabs"][0]["id"], "root");
    assert_eq!(frame["tabs"][0]["name"], "Root Film");
    assert_eq!(frame["window"]["total"], 1);
}

/// A viewer pane shorter than two insets still publishes (fit edge to edge),
/// and a 32 × 18 composition fits a real pane.
#[test]
fn viewer_publishes_for_tiny_panes_and_tiny_compositions() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "viewer-tiny".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "viewer", false).unwrap();
    for stage in [[20., 10.], [1., 1.], [814., 467.]] {
        backend.native.facts["stage"] = json!(stage);
        backend.viewer_snapshot().unwrap();
    }
}

#[test]
fn an_extension_registers_a_command_the_window_can_run() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": "import { jsx } from 'powermove/jsx-runtime';
                import { useState } from 'powermove/hooks';
                let bump = () => {};
                export function activate({ commands }) {
                    commands.register({ id: 'bump', title: 'Bump Count', key: 'cmd+shift+b' }, () => bump());
                }
                export function view() {
                    const [n, setN] = useState(0);
                    bump = () => setN((v) => v + 1);
                    return jsx('text', { children: 'Count ' + n });
                }"
        },
        "edges": {
            "powermove:bundle": {"./index.tsx":"index.tsx"},
            "index.tsx": {
                "powermove/jsx-runtime":"runtime:powermove/jsx-runtime",
                "powermove/hooks":"runtime:powermove/hooks"
            }
        }
    });
    finish(
        panels
            .mount_extension("synthetic-cmd", &graph, None)
            .unwrap(),
    );
    assert!(text_of(&panels, "synthetic-cmd").contains(&"Count 0".to_owned()));
    let (revision, list) = panels.commands_since(0).expect("registered");
    assert_eq!(
        list,
        vec![json!({"panel":"synthetic-cmd","id":"bump","title":"Bump Count","key":"cmd+shift+b"})]
    );
    assert!(panels.commands_since(revision).is_none());
    finish(panels.run_command("synthetic-cmd", "bump", &[]).unwrap());
    assert!(text_of(&panels, "synthetic-cmd").contains(&"Count 1".to_owned()));
    assert!(
        panels
            .run_command("synthetic-cmd", "missing", &[])
            .is_ok_and(|t| {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    if let Some(result) = t.try_take() {
                        break result.is_err();
                    }
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(2));
                }
            })
    );
    // Its commands leave with it.
    panels.unmount_extension("synthetic-cmd");
    let (_, list) = panels.commands_since(revision).expect("removed");
    assert!(list.is_empty());
    panels.close();
    actor.shutdown();
}

/// Properties lists a Masks section: Add Mask adds one, its rows write the
/// mask's channels, and Remove takes it away.
#[test]
fn the_masks_section_adds_edits_and_removes_masks() {
    let actor = actor();
    let kernel = actor.handle();
    let select = SelectionCall::Layers {
        ids: vec!["a".into()],
        add: false,
    };
    kernel
        .call("synthetic", Request::new(Caller::native(), select))
        .wait()
        .unwrap()
        .unwrap();
    let mut model = pm_panel_inspector::Inspector::new(
        kernel.published("synthetic").unwrap(),
        Default::default(),
    );
    let mut presenter = properties::Properties::default();
    let masks = |presenter: &mut properties::Properties,
                 model: &mut pm_panel_inspector::Inspector| {
        let snapshot = presenter.snapshot(model, 0., false).unwrap();
        snapshot["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "masks")
            .expect("masks section")["masks"]
            .as_array()
            .unwrap()
            .clone()
    };
    assert!(masks(&mut presenter, &mut model).is_empty());
    let run = |request: Request| {
        let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
        assert!(
            !matches!(ack.reply, pm_kernel::Reply::Throw(_)),
            "{:?}",
            ack.reply
        );
    };
    let add = json!({"type":"section-action","section":"masks","action":"mask"});
    run(presenter
        .intent(&mut model, Caller::native(), &add, 0.)
        .unwrap()
        .unwrap());
    model.sync(kernel.published("synthetic").unwrap());
    let list = masks(&mut presenter, &mut model);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["name"], "Mask 1");
    let rows = list[0]["rows"].as_array().unwrap();
    let labels: Vec<_> = rows.iter().map(|r| r["label"].as_str().unwrap()).collect();
    assert_eq!(
        labels,
        [
            "Enabled", "Shape", "Mode", "X", "Y", "Width", "Height", "Rotation", "Feather"
        ]
    );
    let feather = rows.iter().find(|r| r["label"] == "Feather").unwrap();
    let handle = feather["control"]["axes"][0]["binding"]["handle"]
        .as_str()
        .unwrap();
    run(presenter
        .write(&mut model, Caller::native(), handle, 12.into(), 0.)
        .unwrap());
    model.sync(kernel.published("synthetic").unwrap());
    let id = list[0]["id"].as_str().unwrap().to_owned();
    let layer = model.layer("a").unwrap();
    let mask = &layer.get("masks").unwrap().as_array().unwrap()[0];
    let feather = mask
        .get("p")
        .and_then(|p| p.get("feather"))
        .and_then(|f| f.get("v"));
    assert_eq!(feather.and_then(pm_model::Value::as_f64), Some(12.));
    let remove = json!({"type":"remove-mask","mask":id});
    run(presenter
        .intent(&mut model, Caller::native(), &remove, 0.)
        .unwrap()
        .unwrap());
    model.sync(kernel.published("synthetic").unwrap());
    assert!(masks(&mut presenter, &mut model).is_empty());
}

#[test]
fn an_extension_offers_a_theme_the_window_can_apply() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": "import { jsx } from 'powermove/jsx-runtime';
                export function activate({ themes }) {
                    themes.register({ id: 'warm', title: 'Warm', tokens: { 'colors.accent': '#ff8800', 'metrics.gap': 8 } });
                    themes.activate('warm');
                }
                export function view() { return jsx('text', { children: 'Themed' }); }"
        },
        "edges": {
            "powermove:bundle": {"./index.tsx":"index.tsx"},
            "index.tsx": {"powermove/jsx-runtime":"runtime:powermove/jsx-runtime"}
        }
    });
    finish(
        panels
            .mount_extension("synthetic-theme", &graph, None)
            .unwrap(),
    );
    let (revision, list) = panels.themes_since(0).expect("registered");
    assert_eq!(
        list,
        vec![json!({"panel":"synthetic-theme","id":"warm","title":"Warm",
            "tokens":{"colors.accent":"#ff8800","metrics.gap":8}})]
    );
    assert!(panels.themes_since(revision).is_none());
    let (_, panel, id) = panels.theme_request().expect("activated");
    assert_eq!(
        (panel.as_str(), id.as_deref()),
        ("synthetic-theme", Some("warm"))
    );
    // Its themes leave with it.
    panels.unmount_extension("synthetic-theme");
    let (_, list) = panels.themes_since(revision).expect("removed");
    assert!(list.is_empty());
    panels.close();
    actor.shutdown();
}
#[test]
fn an_extension_adds_items_to_a_panel_context_menu() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": "import { jsx } from 'powermove/jsx-runtime';
                import { useState } from 'powermove/hooks';
                let mark = () => {};
                export function activate({ menus }) {
                    menus.contribute('panel:context', (ctx) => [
                        { header: 'Synthetic' },
                        { label: 'Mark ' + ctx.panelId, run: () => mark() },
                        '-',
                        { label: 'More', submenu: [{ label: 'Off', disabled: true }] },
                    ]);
                    menus.contribute('panel:context', () => new Promise(() => {}));
                }
                export function view() {
                    const [n, setN] = useState(0);
                    mark = () => setN((v) => v + 1);
                    return jsx('text', { children: 'Marked ' + n });
                }"
        },
        "edges": {
            "powermove:bundle": {"./index.tsx":"index.tsx"},
            "index.tsx": {
                "powermove/jsx-runtime":"runtime:powermove/jsx-runtime",
                "powermove/hooks":"runtime:powermove/hooks"
            }
        }
    });
    finish(
        panels
            .mount_extension("synthetic-menu", &graph, None)
            .unwrap(),
    );
    assert!(panels.has_extensions());
    let asked = Instant::now();
    let gathered = panels.gather_menus("panel:context", &json!({"panelId":"timeline"}));
    let (panel, ticket) = gathered.into_iter().next().unwrap();
    let value = finish(ticket.unwrap()).value;
    // A contribution that never answers is left out after 100 ms.
    assert!(asked.elapsed() < Duration::from_secs(2));
    let text: String = serde_json::from_str::<Value>(&value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or(value);
    let items: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(panel, "synthetic-menu");
    assert_eq!(items[0], json!({"header":"Synthetic"}));
    assert_eq!(items[1]["label"], "Mark timeline");
    assert_eq!(items[2], "-");
    assert_eq!(items[3]["submenu"][0]["disabled"], true);
    let key = items[1]["key"].as_str().unwrap().to_owned();
    finish(panels.run_menu("synthetic-menu", &key).unwrap());
    assert!(text_of(&panels, "synthetic-menu").contains(&"Marked 1".to_owned()));
    panels.unmount_extension("synthetic-menu");
    assert!(!panels.has_extensions());
    panels.close();
    actor.shutdown();
}
#[test]
fn an_extension_draws_a_viewer_overlay_that_follows_the_frame() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": "import { jsx } from 'powermove/jsx-runtime';
                export function activate({ overlays }) {
                    overlays.viewer(({ frame }) => jsx('div', { style: { position: 'absolute',
                        left: frame.x, width: frame.width, background: '#5a1f8c' },
                        children: jsx('text', { children: 'Frame ' + frame.width }) }));
                }
                export function view() { return jsx('text', { children: 'Panel' }); }"
        },
        "edges": {
            "powermove:bundle": {"./index.tsx":"index.tsx"},
            "index.tsx": {"powermove/jsx-runtime":"runtime:powermove/jsx-runtime"}
        }
    });
    finish(
        panels
            .mount_extension("synthetic-overlay", &graph, None)
            .unwrap(),
    );
    let overlay = "synthetic-overlay@viewer";
    let text = |panels: &Builtins| -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(p) = panels
                .take_publications()
                .into_iter()
                .find(|p| p.panel == overlay)
            {
                return p.surface.nodes().values().map(|n| n.text.clone()).collect();
            }
            assert!(Instant::now() < deadline, "no overlay publication");
            thread::sleep(Duration::from_millis(2));
        }
    };
    assert!(text(&panels).contains(&"Frame 0".to_owned()));
    panels.viewer_frame(&json!({"x":10,"y":20,"width":640,"height":360,"scale":0.5}));
    assert!(text(&panels).contains(&"Frame 640".to_owned()));
    // The same frame again draws nothing: a still overlay stays idle.
    panels.viewer_frame(&json!({"x":10,"y":20,"width":640,"height":360,"scale":0.5}));
    thread::sleep(Duration::from_millis(200));
    assert!(
        panels
            .take_publications()
            .iter()
            .all(|p| p.panel != overlay)
    );
    panels.unmount_extension("synthetic-overlay");
    assert_eq!(panels.take_cleared_overlays(), vec![overlay.to_owned()]);
    panels.close();
    actor.shutdown();
}
#[test]
fn an_extension_paints_on_a_thousand_timeline_clips() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": "import { jsx } from 'powermove/jsx-runtime';
                export function activate({ timeline }) {
                    timeline.paint((clips) => Object.fromEntries(clips.map((c) =>
                        [c.id, [{ rect: [0, 0, c.width, c.height], color: '#5a1f8c', opacity: 0.5 }]])));
                }
                export function view() { return jsx('text', { children: 'Painter' }); }"
        },
        "edges": {
            "powermove:bundle": {"./index.tsx":"index.tsx"},
            "index.tsx": {"powermove/jsx-runtime":"runtime:powermove/jsx-runtime"}
        }
    });
    assert_eq!(panels.painters_revision(), 0, "no painter yet");
    finish(
        panels
            .mount_extension("synthetic-painter", &graph, None)
            .unwrap(),
    );
    assert_ne!(panels.painters_revision(), 0);
    let clips: Vec<Value> = (0..1000)
        .map(|i| json!({"id":format!("L{i}"),"layerId":format!("L{i}"),"width":80,"height":20,"from":0,"duration":1}))
        .collect();
    let started = Instant::now();
    let (_, ticket) = panels
        .paint_clips(&json!(clips))
        .into_iter()
        .next()
        .unwrap();
    let text = finish(ticket.unwrap()).value;
    eprintln!(
        "JS painter for 1,000 clips (debug, off the main thread): {:?}",
        started.elapsed()
    );
    let text: String = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or(text);
    let shapes: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(shapes.as_object().unwrap().len(), 1000);
    assert_eq!(shapes["L7"][0]["color"], "#5a1f8c");
    panels.close();
    actor.shutdown();
}
#[test]
fn a_closed_panel_parks_its_isolate_and_starts_again_when_opened() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("fxbrowser").unwrap());
    assert!(!panels.is_parked("fxbrowser"));
    // The viewer and timeline never park; a panel that isn't running can't.
    assert!(!panels.park("viewer"));
    assert!(panels.park("fxbrowser"));
    assert!(panels.is_parked("fxbrowser"));
    assert!(panels.unpark("fxbrowser").expect("parked").is_ok());
    assert!(!panels.is_parked("fxbrowser"));
    assert!(panels.unpark("fxbrowser").is_none());
    panels.close();
    actor.shutdown();
}

/// A synthetic v4 extension's module graph from its `index.tsx`.
fn graph_of(index: &str) -> Value {
    json!({
        "sources": {
            "powermove:bundle": "export * from './index.tsx';",
            "index.tsx": format!("import {{ jsx }} from 'powermove/jsx-runtime';\n{index}"),
        },
        "edges": {
            "powermove:bundle": {"./index.tsx":"index.tsx"},
            "index.tsx": {"powermove/jsx-runtime":"runtime:powermove/jsx-runtime"}
        }
    })
}
/// The project's published `chrome` once `done` holds for it.
fn chrome_when(actor: &KernelActor, done: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let chrome = actor
            .handle()
            .published("synthetic")
            .and_then(|p| {
                serde_json::from_str::<Value>(&pm_model::json::to_canonical_string(&p.workspace))
                    .ok()
            })
            .map_or(Value::Null, |w| w["chrome"].clone());
        if done(&chrome) {
            return chrome;
        }
        assert!(
            Instant::now() < deadline,
            "chrome never became right: {chrome}"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn overlay_text(panels: &Builtins, overlay: &str, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if panels
            .take_publications()
            .into_iter()
            .any(|p| p.panel == overlay && p.surface.nodes().values().any(|n| n.text == want))
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{overlay} never showed {want}: {:?}",
            panels.take_errors()
        );
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn timeline_chrome_edits_stack_per_extension_and_none_is_lost() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    // Two edits in a row: the second must not replace the first.
    let a = graph_of(
        "let refused = '';
        export function activate({ timeline }) {
            timeline.chrome({ rowHeight: 40 });
            timeline.chrome({ gutterWidth: 340 });
            try { timeline.chrome({ showTypeBadges: false }); } catch (e) { refused = String(e.message ?? e); }
        }
        export const view = () => jsx('text', { children: 'Refused ' + refused });",
    );
    let b = graph_of(
        "export function activate({ timeline }) { timeline.chrome({ rowHeight: 30, surfaceOrder: 'reversed' }); }
        export const view = () => jsx('text', { children: 'B' });",
    );
    finish(panels.mount_extension("chrome-a", &a, None).unwrap());
    assert!(
        text_of(&panels, "chrome-a")
            .iter()
            .any(|t| t.contains("aren't drawn by the native timeline")),
        "fields the native timeline can't draw are refused"
    );
    chrome_when(&actor, |c| {
        c["timeline"] == json!({"rowHeight":40,"gutterWidth":340})
    });
    finish(panels.mount_extension("chrome-b", &b, None).unwrap());
    chrome_when(&actor, |c| {
        c["timeline"] == json!({"rowHeight":30,"gutterWidth":340,"surfaceOrder":"reversed"})
            && c["timelineSurfaceOrder"] == "reversed"
    });
    // A leaving under B: B's row height stays; A's gutter goes back.
    panels.unmount_extension("chrome-a");
    chrome_when(&actor, |c| {
        c["timeline"] == json!({"rowHeight":30,"surfaceOrder":"reversed"})
    });
    // B leaving puts back the original, sibling field included.
    panels.unmount_extension("chrome-b");
    chrome_when(&actor, |c| {
        c["timeline"] == json!({}) && c.get("timelineSurfaceOrder").is_none()
    });
    panels.close();
    actor.shutdown();
}
#[test]
fn a_failed_generation_takes_down_its_overlay_chrome_and_painter() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = graph_of(
        "export function activate({ overlays, timeline, commands }) {
            overlays.viewer(() => jsx('text', { children: 'Shown' }));
            timeline.chrome({ rowHeight: 44 });
            timeline.paint(() => ({}));
            commands.register({ id: 'blow', title: 'Blow' }, () => {
                globalThis.kept = new Uint8Array(64 * 1024 * 1024);
            });
        }
        export const view = () => jsx('text', { children: 'Failing' });",
    );
    finish(panels.mount_extension("failing", &graph, None).unwrap());
    overlay_text(&panels, "failing@viewer", "Shown");
    chrome_when(&actor, |c| c["timeline"]["rowHeight"] == 44);
    assert_ne!(panels.painters_revision(), 0);
    // Over the isolate's external-memory quota: the generation fails.
    let deadline = Instant::now() + Duration::from_secs(20);
    let ticket = panels.run_command("failing", "blow", &[]).unwrap();
    let failed = loop {
        if let Some(result) = ticket.try_take() {
            break result;
        }
        assert!(Instant::now() < deadline, "the command never ended");
        thread::sleep(Duration::from_millis(2));
    };
    assert!(failed.is_err(), "{failed:?}");
    chrome_when(&actor, |c| c["timeline"].get("rowHeight").is_none());
    assert!(
        panels
            .take_cleared_overlays()
            .contains(&"failing@viewer".to_owned())
    );
    assert_eq!(panels.painters_revision(), 0);
    panels.close();
    actor.shutdown();
}
#[test]
fn a_fork_parks_as_itself_and_gets_painters_overlays_and_menus() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let fork = graph_of(
        "export function activate({ overlays, timeline, menus }) {
            overlays.viewer(({ frame }) => jsx('text', { children: 'Fork frame ' + frame.width }));
            timeline.paint((clips) => Object.fromEntries(clips.map((c) => [c.id, [{ rect: [0, 0, 4, 4], color: '#5a1f8c' }]])));
            menus.contribute('layer:context', () => [{ label: 'Fork item', run: () => {} }]);
        }
        export const view = () => jsx('text', { children: 'Fork view' });",
    );
    finish(panels.mount_builtin("inspector").unwrap());
    text_of(&panels, "inspector");
    finish(
        panels
            .replace("inspector", "inspector-fork", &fork)
            .unwrap(),
    );
    assert!(text_of(&panels, "inspector").contains(&"Fork view".to_owned()));
    assert!(panels.has_extensions());
    // Painters and overlay frames reach a fork, not only third-party realms.
    assert_ne!(panels.painters_revision(), 0);
    let painted = panels.paint_clips(&json!([{"id":"L1"}]));
    assert_eq!(painted.len(), 1);
    let (panel, ticket) = painted.into_iter().next().unwrap();
    assert_eq!(panel, "inspector");
    assert!(finish(ticket.unwrap()).value.contains("#5a1f8c"));
    panels.viewer_frame(&json!({"x":0,"y":0,"width":320,"height":180,"scale":1}));
    overlay_text(&panels, "inspector@viewer", "Fork frame 320");
    let (_, ticket) = panels
        .gather_menus("layer:context", &json!({}))
        .into_iter()
        .next()
        .expect("the fork contributes");
    assert!(finish(ticket.unwrap()).value.contains("Fork item"));
    // Closed and opened again, it is still the fork.
    assert!(panels.park("inspector"));
    assert_eq!(panels.painters_revision(), 0, "its painter left with it");
    finish(panels.unpark("inspector").expect("parked").unwrap());
    assert!(text_of(&panels, "inspector").contains(&"Fork view".to_owned()));
    // Restored, the shipped panel comes back and user realms are gone.
    finish(panels.restore("inspector").unwrap());
    assert!(!text_of(&panels, "inspector").contains(&"Fork view".to_owned()));
    assert!(!panels.has_extensions());
    panels.close();
    actor.shutdown();
}
#[test]
fn the_latest_theme_activation_wins_and_a_repeat_counts() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let theme = |id: &str, colour: &str| {
        graph_of(&format!(
            "export function activate({{ themes, commands }}) {{
                themes.register({{ id: '{id}', title: '{id}', tokens: {{ 'colors.accent': '{colour}' }} }});
                themes.activate('{id}');
                commands.register({{ id: 'again', title: 'Again' }}, () => themes.activate('{id}'));
            }}
            export const view = () => jsx('text', {{ children: '{id}' }});"
        ))
    };
    // `aa` sorts first but activated first: `zz`, activated later, wins.
    finish(
        panels
            .mount_extension("aa", &theme("red", "#ff0000"), None)
            .unwrap(),
    );
    finish(
        panels
            .mount_extension("zz", &theme("blue", "#0000ff"), None)
            .unwrap(),
    );
    let (first, panel, id) = panels.theme_request().unwrap();
    assert_eq!((panel.as_str(), id.as_deref()), ("zz", Some("blue")));
    finish(panels.run_command("aa", "again", &[]).unwrap());
    let (second, panel, id) = panels.theme_request().unwrap();
    assert_eq!((panel.as_str(), id.as_deref()), ("aa", Some("red")));
    assert!(second > first);
    finish(panels.run_command("aa", "again", &[]).unwrap());
    assert!(
        panels.theme_request().unwrap().0 > second,
        "a repeat is a new request"
    );
    panels.close();
    actor.shutdown();
}
#[test]
fn a_replaced_viewer_overlay_publishes_from_its_own_first_commit() {
    let actor = actor();
    let panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    let graph = graph_of(
        "export function activate({ overlays, commands }) {
            overlays.viewer(() => jsx('text', { children: 'First' }));
            commands.register({ id: 'swap', title: 'Swap' }, () => {
                overlays.viewer(() => jsx('text', { children: 'Second' }));
            });
        }
        export const view = () => jsx('text', { children: 'Overlay owner' });",
    );
    finish(panels.mount_extension("swapper", &graph, None).unwrap());
    overlay_text(&panels, "swapper@viewer", "First");
    finish(panels.run_command("swapper", "swap", &[]).unwrap());
    overlay_text(&panels, "swapper@viewer", "Second");
    // The first root's teardown never lands on the second.
    thread::sleep(Duration::from_millis(200));
    for p in panels.take_publications() {
        assert!(
            p.panel != "swapper@viewer" || p.surface.nodes().values().any(|n| n.text == "Second")
        );
    }
    assert!(panels.take_errors().iter().all(|e| !e.contains("revision")));
    panels.close();
    actor.shutdown();
}

#[test]
fn project_size_offers_the_shipped_presets_and_writes_width_and_height() {
    let actor = actor();
    let kernel = actor.handle();
    let identity = pm_surface::host_api::Identity {
        realm: "settings-size".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "settings", false).unwrap();
    let publication = backend.settings_snapshot().unwrap();
    let rows = publication["rows"].as_array().unwrap();
    let find = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap().clone();
    let size = find("project.size");
    assert!(size["binding"].is_string(), "{size}");
    assert_eq!(
        size["options"][0],
        json!({"v":"3840x2160","label":"4K UHD · 3840×2160"})
    );
    assert!(find("project.width")["binding"].is_string());
    assert!(find("project.height")["binding"].is_string());
    assert!(backend.native_command("panel.settings", &json!({"revision":publication["revision"],"action":{"type":"write","id":"project.size","value":"custom"}})).is_err());
    let models::NativeCommand::Kernel(request) = backend.native_command("panel.settings", &json!({"revision":publication["revision"],"action":{"type":"write","id":"project.size","value":"1280x720"}})).unwrap() else { panic!("actor request") };
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let publication = backend.settings_snapshot().unwrap();
    let rows = publication["rows"].as_array().unwrap();
    let find = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap().clone();
    assert_eq!(find("project.size")["value"], json!("1280x720"));
    assert_eq!(find("project.width")["value"], json!(1280.0));
    assert_eq!(find("project.height")["value"], json!(720.0));
}

/// Settings ▸ Project ▸ Export defaults: real controls that write the project's
/// `exportDefaults` through the kernel (one undo), and open the Export sheet.
#[test]
fn export_defaults_rows_write_the_project_and_open_the_export_sheet() {
    let actor = actor();
    let kernel = actor.handle();
    let identity = pm_surface::host_api::Identity {
        realm: "settings-export".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "settings", false).unwrap();
    let row = |p: &Value, id: &str| {
        p["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("{id}"))
            .clone()
    };
    let publication = backend.settings_snapshot().unwrap();
    for id in [
        "project.format",
        "project.resolution",
        "project.export-fps",
        "project.range",
        "project.quality",
        "project.motion-blur",
        "project.audio",
        "project.alpha",
    ] {
        let r = row(&publication, id);
        assert_eq!(r["group"], "Export defaults");
        assert!(r["binding"].is_string(), "{id} has a control binding");
        if r["control"] == "choice" {
            assert!(
                !r["options"].as_array().unwrap().is_empty(),
                "{id} offers choices"
            );
        }
    }
    assert_eq!(row(&publication, "project.format")["value"], "mp4");
    // An MP4 has no alpha: the row stays, dimmed.
    assert_eq!(row(&publication, "project.alpha")["disabled"], true);
    for (id, value) in [
        ("project.format", json!("gif")),
        ("project.audio", json!("yes")),
    ] {
        assert!(backend.native_command("panel.settings", &json!({"revision":publication["revision"],"action":{"type":"write","id":id,"value":value}})).is_err());
    }
    let models::NativeCommand::Kernel(request) = backend.native_command("panel.settings", &json!({"revision":publication["revision"],"action":{"type":"write","id":"project.format","value":"prores"}})).unwrap() else { panic!("actor request") };
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    let stored = |kernel: &pm_kernel::KernelHandle| {
        kernel
            .published("synthetic")
            .unwrap()
            .snapshot
            .root()
            .unwrap()
            .get("exportDefaults")
            .map(|n| n.to_value())
    };
    let defaults = stored(&kernel).expect("exportDefaults written");
    assert_eq!(
        defaults.get("format"),
        Some(&pm_model::Value::from("prores"))
    );
    let publication = backend.settings_snapshot().unwrap();
    assert_eq!(row(&publication, "project.format")["value"], "prores");
    assert_eq!(row(&publication, "project.alpha")["disabled"], false);
    // The Export sheet opens with the project's defaults.
    let mut sheet = data::Backend::new(
        kernel.clone(),
        pm_surface::host_api::Identity {
            realm: "export-defaults".into(),
            generation: 1,
            project: "synthetic".into(),
        },
        "export",
        false,
    )
    .unwrap();
    assert_eq!(
        sheet.export_snapshot().unwrap()["options"]["format"],
        "prores"
    );
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(stored(&kernel), None, "one undo restores the project");
}
