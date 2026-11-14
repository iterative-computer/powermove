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
fn a_panic_holding_a_panel_lock_never_stops_the_v8_owner_thread() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("layer-effects").unwrap());
    panels.take_publications();
    // A host thread panics while it holds the panel's backend.
    let backend = panels
        .broker
        .entry("layer-effects")
        .unwrap()
        .lock()
        .unwrap()
        .backend
        .clone();
    let held = backend.clone();
    let panicked = thread::spawn(move || {
        let _guard = held.lock().unwrap();
        panic!("a host thread fails while holding panel state");
    })
    .join();
    assert!(panicked.is_err() && backend.is_poisoned());
    // The owner thread still serves the panel: a selection republishes it.
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
    assert!(!backend.is_poisoned());
    assert!(panels.take_errors().is_empty());
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
fn every_authored_entry_mounts_and_publishes_an_accessible_native_tree() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    for panel in BUILTIN_PANELS.iter() {
        let id = &panel.id;
        let ticket = panels.mount(id).unwrap();
        finish(ticket);
        if panel.window {
            finish(panels.show(id).unwrap());
        }
    }
    let publications = panels.take_publications();
    assert_eq!(publications.len(), BUILTIN_PANELS.len());
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
fn bring_my_workspace_opens_a_thread_with_the_request_and_sends_the_agent_its_steps() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let mut panels = Builtins::application(actor.handle(), || {}, None).unwrap();
    // The Agent panel is what carries it: closed, there is nowhere to ask.
    assert!(panels.bring_workspace().is_err());
    finish(panels.mount("agent").unwrap());
    finish(panels.show("agent").unwrap());
    let agent = {
        let backend = panels.backend("agent").unwrap();
        let backend = backend.shared();
        backend.native.agent.clone().unwrap()
    };
    // No agent is signed in: the request waits in the composer of the
    // visible thread (a blank one is reused, as New Thread reuses it).
    assert_eq!(panels.bring_workspace(), Ok(false));
    {
        let state = agent.shared();
        let thread = state.panel.active_id();
        let session = state.panel.session(thread).unwrap();
        assert_eq!(session.draft.text(), agent_host::WORKSPACE_IMPORT_LABEL);
        assert_eq!(session.settings.access, pm_agents::Access::Project);
        assert!(session.run.is_none());
    }
    // Sent, the conversation shows the one line and the agent gets the steps.
    let sent = agent_host::expand_request(agent_host::WORKSPACE_IMPORT_LABEL);
    assert!(sent.starts_with(agent_host::WORKSPACE_IMPORT_LABEL));
    for tool in [
        "inspect_creative_workspace",
        "inspect_creative_extension",
        "get_panel_layout",
        "set_panel_layout",
    ] {
        assert!(sent.contains(tool), "{tool}");
    }
    assert_eq!(agent_host::expand_request("Make it blue"), "Make it blue");
    panels.close();

    // The Settings row's button hands the transfer to the app's window.
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "builtin-application".into(),
    };
    let mut settings = data::Backend::new(actor.handle(), identity, "settings", true).unwrap();
    let revision = settings.settings_snapshot().unwrap()["revision"].clone();
    settings
        .native_command(
            "panel.settings",
            &json!({"revision":revision,"action":{"type":"row","id":"workspace","action":"workspace.import"}}),
        )
        .expect("the button no longer ends in an error");
    assert!(
        settings
            .native
            .platform
            .iter()
            .any(|(kind, request)| kind == "command" && request["name"] == "bringWorkspace")
    );
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
    // The hex field beside the swatch is its own lease on the same property:
    // typed hex reaches the layer, and the row shows it.
    let publication = presenter.snapshot(&mut model, 0., false).unwrap();
    let colour = publication["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap())
        .find(|r| r["control"]["kind"] == "colour")
        .expect("colour row")
        .clone();
    assert_eq!(colour["control"]["hex"], "#ABCDEF");
    let hex = colour["control"]["hexBinding"]["handle"].as_str().unwrap();
    assert_ne!(
        hex,
        colour["control"]["binding"]["handle"].as_str().unwrap()
    );
    let request = presenter
        .write(&mut model, Caller::native(), hex, "#FF5A36".into(), 0.)
        .unwrap();
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    model.sync(kernel.published("synthetic").unwrap());
    let text = pm_model::json::to_canonical_string(&model.layer("a").unwrap().to_owned());
    assert!(text.contains("#FF5A36"), "{text}");
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
fn the_distribute_buttons_space_three_selected_layers_as_one_undo_step() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let layer = |id: &str, x: f64| {
        json!({"id":id,"type":"shape","name":id,"from":0,"dur":10,"on":true,"lock":false,
        "p":{"opacity":{"v":100,"kf":[]},"position.x":{"v":x,"kf":[]},"position.y":{"v":180,"kf":[]}},
        "d":{"w":40,"h":40},"fx":[]})
    };
    let project = json!({"id":"synthetic","name":"Synthetic","compId":"c","w":640,"h":360,"fps":30,"dur":10,
        "bg":"#000000","assets":{},"comps":{},"layers":[layer("a",100.),layer("b",150.),layer("c",500.)]});
    let kernel = actor.handle();
    kernel
        .open(
            "synthetic",
            OpenSource::Untitled(pm_model::json::parse(&project.to_string()).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    let select = |ids: &[&str]| {
        kernel
            .call(
                "synthetic",
                Request::new(
                    Caller::native(),
                    SelectionCall::Layers {
                        ids: ids.iter().map(|id| (*id).to_owned()).collect(),
                        add: false,
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
    };
    let identity = pm_surface::host_api::Identity {
        realm: "inspector".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "inspector", false).unwrap();
    let press = |backend: &mut data::Backend| {
        backend.native_command(
            "panel.inspector",
            &json!({"type":"distribute","axis":"horizontal"}),
        )
    };
    // Two layers: nothing to distribute, and the panel says why.
    select(&["a", "b"]);
    backend.inspector_snapshot().unwrap();
    assert!(press(&mut backend).is_err());
    select(&["a", "b", "c"]);
    backend.inspector_snapshot().unwrap();
    let Ok(models::NativeCommand::Kernel(request)) = press(&mut backend) else {
        panic!("the button edits the project")
    };
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let published = kernel.published("synthetic").unwrap();
    let x = |id: &str| {
        pm_kernel::QueryMirror::new(&published)
            .queries()
            .evaluate(None, id, "position.x", 0.)
            .unwrap()
            .as_f64()
            .unwrap()
    };
    // The outer two stay; the middle one takes the midpoint.
    assert_eq!((x("a"), x("b"), x("c")), (100., 300., 500.));
    assert_eq!(
        published.history.undo_label.as_deref(),
        Some("Distribute Layers")
    );
}

/// Properties ▸ Time (`RetimingSection.svelte`): Time Remap, Freeze Frame
/// and Reverse on a video layer, then its remapping rows.
#[test]
fn the_time_section_remaps_freezes_and_reverses_a_video_layer() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let project = json!({"id":"synthetic","name":"Synthetic","compId":"c","w":640,"h":360,"fps":30,"dur":10,
        "bg":"#000000","assets":{"v1":{"id":"v1","kind":"video","name":"clip.mov","dur":20}},"comps":{},
        "layers":[
            {"id":"clip","type":"video","name":"Clip","from":2,"dur":3,"on":true,
             "d":{"asset":"v1","fit":"cover","trim":5,"speed":1,"w":64,"h":36},"p":{"opacity":{"v":100,"kf":[]}},"fx":[]},
            {"id":"box","type":"shape","name":"Box","from":0,"dur":10,"on":true,
             "d":{"w":40,"h":40},"p":{"opacity":{"v":100,"kf":[]}},"fx":[]}]});
    let kernel = actor.handle();
    kernel
        .open(
            "synthetic",
            OpenSource::Untitled(pm_model::json::parse(&project.to_string()).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    let select = |id: &str| {
        kernel
            .call(
                "synthetic",
                Request::new(
                    Caller::native(),
                    SelectionCall::Layers {
                        ids: vec![id.to_owned()],
                        add: false,
                    },
                ),
            )
            .wait()
            .unwrap()
            .unwrap();
    };
    let identity = pm_surface::host_api::Identity {
        realm: "inspector".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "inspector", false).unwrap();
    let time_section = |backend: &mut data::Backend| {
        let snapshot = backend.inspector_snapshot().unwrap();
        snapshot["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "time")
            .cloned()
    };
    // A shape has no Time section.
    select("box");
    assert!(time_section(&mut backend).is_none());
    select("clip");
    let section = time_section(&mut backend).expect("a video layer has Time");
    assert_eq!(section["title"], "Time");
    assert_eq!(
        section["actions"],
        json!(["time-remap", "freeze-frame", "reverse"])
    );
    assert_eq!(
        section["rows"],
        json!([]),
        "no remapping rows before remapping"
    );
    let Ok(models::NativeCommand::Kernel(request)) = backend.native_command(
        "panel.inspector",
        &json!({"type":"section-action","section":"time","action":"freeze-frame"}),
    ) else {
        panic!("Freeze Frame edits the project")
    };
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let published = kernel.published("synthetic").unwrap();
    assert_eq!(
        published.history.undo_label.as_deref(),
        Some("Freeze Frame")
    );
    let root = published.snapshot.root().unwrap();
    let at = pm_kernel::model::find_layer(root, "clip").unwrap();
    let clip = pm_kernel::model::layer_at(root, &at).unwrap().to_value();
    let keys = clip
        .get("d")
        .and_then(|d| d.get("sourceTime"))
        .and_then(|s| s.get("kf"));
    assert_eq!(
        keys.and_then(pm_model::Value::as_array).map(Vec::len),
        Some(1)
    );
    let section = time_section(&mut backend).unwrap();
    let rows: Vec<&str> = section["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["label"].as_str())
        .collect();
    assert_eq!(rows, ["Time Remap", "Source Time"]);
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
fn the_export_sheet_opens_on_the_projects_output() {
    let actor = actor();
    let kernel = actor.handle();
    let backend = |realm: &str, panel: &str| {
        let identity = pm_surface::host_api::Identity {
            realm: realm.into(),
            generation: 1,
            project: "synthetic".into(),
        };
        data::Backend::new(kernel.clone(), identity, panel, false).unwrap()
    };
    let opened = |realm: &str| {
        let mut export = backend(realm, "export");
        let snapshot = export.export_snapshot().unwrap();
        // Absent is SDR (options leave the default out).
        let range = snapshot["options"]["dynamicRange"]
            .as_str()
            .unwrap_or("sdr");
        (json!(range), snapshot["project"]["hdr"].clone())
    };
    assert_eq!(opened("export-sdr"), (json!("sdr"), json!(false)));
    let mut settings = backend("settings-output", "settings");
    for (choice, realm) in [("hlg", "export-hlg"), ("pq", "export-pq")] {
        let revision = settings.settings_snapshot().unwrap()["revision"].clone();
        let models::NativeCommand::Kernel(request) = settings
            .native_command(
                "panel.settings",
                &json!({"revision":revision,"action":{"type":"write","id":"project.output","value":choice}}),
            )
            .unwrap()
        else {
            panic!("actor request")
        };
        kernel.call("synthetic", request).wait().unwrap().unwrap();
        assert_eq!(opened(realm), (json!(choice), json!(true)), "{choice}");
    }
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

/// An export whose destination is chosen and that waits to start (for the
/// window's extensions to load) is shown as preparing: its panel's Cancel is
/// live and reaches the coordinator. While the destination is being chosen
/// the form's Cancel is off, so a wait left in that phase could not be
/// cancelled from the panel.
#[test]
fn an_export_waiting_to_start_is_cancelled_from_its_panel() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("export").unwrap());
    finish(panels.show("export").unwrap());
    let shown = std::cell::RefCell::new(None::<Publication>);
    // Presses the panel's control `id` as the view does: a disabled one
    // sends nothing. Returns whether it was pressed.
    let press = |panels: &Builtins, id: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(p) = panels
                .take_publications()
                .into_iter()
                .rfind(|p| p.panel == "export")
            {
                *shown.borrow_mut() = Some(p);
            }
            let p = shown.borrow().clone();
            let found = p.as_ref().and_then(|p| {
                p.surface.nodes().iter().find_map(|(target, n)| {
                    (n.props.get("id").and_then(Value::as_str) == Some(id))
                        .then(|| (*target, n.props.get("disabled") == Some(&Value::Bool(true))))
                })
            });
            if let (Some(p), Some((target, disabled))) = (p, found) {
                if disabled {
                    return false;
                }
                finish(
                    panels
                        .activate(
                            "export",
                            p.generation,
                            p.surface.revision(),
                            target,
                            Some(id),
                            "click",
                            Default::default(),
                        )
                        .unwrap(),
                );
                return true;
            }
            assert!(Instant::now() < deadline, "no {id} in the export panel");
            thread::sleep(Duration::from_millis(2));
        }
    };
    // What the panel asked of the coordinator so far; waits up to `wait`
    // for `operation` to be among it.
    let seen = std::cell::RefCell::new(Vec::<String>::new());
    let asked = |panels: &Builtins, operation: &str, wait: Duration| {
        let deadline = Instant::now() + wait;
        loop {
            seen.borrow_mut().extend(
                panels
                    .take_platform_requests()
                    .into_iter()
                    .map(|r| r.operation),
            );
            if seen.borrow().iter().any(|o| o == operation) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(2));
        }
    };
    assert!(press(&panels, "export.start"));
    assert!(
        asked(&panels, "export.begin", Duration::from_secs(10)),
        "{:?} {:?}",
        seen.borrow(),
        panels.take_errors()
    );
    // Choosing the destination: the form's Cancel is off.
    assert!(!press(&panels, "export.cancel"), "Cancel while choosing");
    assert!(!asked(&panels, "export.cancel", Duration::from_millis(300)));
    // The destination is chosen and the export waits: the coordinator says
    // so before it waits, and Cancel reaches it.
    finish(panels.observe_export(json!({"phase":"preparing"})).unwrap());
    thread::sleep(Duration::from_millis(100));
    assert!(press(&panels, "export.cancel"), "Cancel while it waits");
    assert!(asked(&panels, "export.cancel", Duration::from_secs(10)));
    assert!(panels.take_errors().is_empty());
}

#[test]
fn a_finished_export_reopens_as_a_fresh_sheet() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("export").unwrap());
    finish(panels.show("export").unwrap());
    panels.take_publications();
    // The coordinator's observations, in its order: progress up to the last
    // frame, then the terminal run once the file is written.
    for run in [
        json!({"phase":"preparing"}),
        json!({"phase":"rendering","frame":150,"total":300,"elapsed":1}),
        json!({"phase":"rendering","frame":300,"total":300,"elapsed":2}),
        json!({"phase":"done","path":"/tmp/Synthetic Film.mp4"}),
    ] {
        finish(panels.observe_export(run).unwrap());
    }
    panels.take_publications();
    finish(panels.show("export").unwrap());
    let text = text_of(&panels, "export");
    assert!(!text.iter().any(|t| t.starts_with("Exporting")), "{text:?}");
    assert!(text.iter().any(|t| t == "Export"), "{text:?}");
    assert!(panels.take_errors().is_empty());
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
fn the_output_setting_reaches_the_kernel_and_unlocks_its_peak() {
    use pm_protocol::color::ProjectOutput;
    let actor = actor();
    let kernel = actor.handle();
    let identity = pm_surface::host_api::Identity {
        realm: "settings-output".into(),
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
    let output = row(&publication, "project.output");
    assert_eq!(output["value"], json!("sdr"));
    assert_eq!(output["control"], json!("choice"));
    assert_eq!(
        output["options"],
        json!([{"v":"sdr","label":"SDR Rec. 709"},{"v":"pq","label":"HDR PQ"},{"v":"hlg","label":"HDR HLG"}])
    );
    assert_eq!(
        row(&publication, "project.masteringPeak")["disabled"],
        json!(true)
    );
    let mut revision = publication["revision"].clone();
    for (id, choice) in [
        ("hlg", ProjectOutput::Hlg),
        ("pq", ProjectOutput::Pq),
        ("sdr", ProjectOutput::Sdr),
    ] {
        let models::NativeCommand::Kernel(request) = backend.native_command("panel.settings", &json!({"revision":revision,"action":{"type":"write","id":"project.output","value":id}})).unwrap() else { panic!("actor request") };
        let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
        assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
        let root = kernel.published("synthetic").unwrap();
        let root = root.snapshot.root().unwrap();
        assert_eq!(
            pm_render::composite::project_color(root).unwrap().output(),
            choice
        );
        let publication = backend.settings_snapshot().unwrap();
        revision = publication["revision"].clone();
        assert_eq!(row(&publication, "project.output")["value"], json!(id));
        assert_eq!(
            row(&publication, "project.masteringPeak")["disabled"],
            json!(!choice.hdr())
        );
        if choice.hdr() {
            assert_eq!(row(&publication, "project.colorDepth")["value"], json!(32));
        }
    }
    assert!(backend.native_command("panel.settings", &json!({"revision":revision,"action":{"type":"write","id":"project.output","value":"aces"}})).is_err());
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

/// One set of play controls on screen: with a timeline in the workspace the
/// viewer publishes no transport (no play buttons, no timecode); without one,
/// it carries both. Unknown (before the first layout) keeps the viewer's.
#[test]
fn the_viewer_shows_transport_only_when_no_timeline_is_shown() {
    let actor = actor();
    let kernel = actor.handle();
    let identity = pm_surface::host_api::Identity {
        realm: "viewer-transport".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "viewer", false).unwrap();
    assert!(backend.viewer_snapshot().unwrap()["host"]["transport"].is_object());
    backend.native.facts["timelineShown"] = json!(true);
    assert!(backend.viewer_snapshot().unwrap()["host"]["transport"].is_null());
    backend.native.facts["timelineShown"] = json!(false);
    let shown = backend.viewer_snapshot().unwrap();
    assert!(shown["host"]["transport"]["timecode"].is_string());
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
            &Default::default(),
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
    let ack=panels.admit_media(&catalog.stamp,vec![RetainedAsset{id:"worker-temp-id".into(),metadata:data::model(&json!({"name":"Relinked.bin","kind":"image","mimeType":"application/octet-stream","w":100,"h":50})).unwrap(),original:replacement,previous:Some(prior)}],&Default::default(),Some("imported-image".into()),vec![],Caller::native()).unwrap().wait().unwrap().unwrap();
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
                &Default::default(),
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
    // Settings shows one section per page: open Memory from the sidebar.
    let first = panels.take_publications().pop().unwrap();
    let nav = first
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("id").and_then(Value::as_str) == Some("settings.nav.memory"))
        .unwrap()
        .id;
    finish(
        panels
            .event(
                "settings",
                first.generation,
                first.surface.revision(),
                nav,
                "click",
                EventDetail::default(),
            )
            .unwrap(),
    );
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
mod expressions;
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

/// View ▸ Timeline in Tracks, Hide Shy Layers, the graph and the reveal keys
/// change the timeline's saved view from outside its panel. The panel's
/// project subscription moves with that view, so its header and rows follow.
#[test]
fn the_timeline_panel_hears_of_a_view_change_made_outside_it() {
    use pm_kernel::state::{StateOp, StateTransaction};
    use pm_surface::host_data::KernelData;
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let project = pm_model::json::parse(r##"{"id":"view-film","name":"View Film","compId":"c","w":32,"h":18,"fps":30,"dur":2,"assets":{},"comps":{},"layers":[{"id":"a","type":"shape","name":"Box","from":0,"dur":2,"on":true,"shy":true,"p":{"opacity":{"v":100,"kf":[]}},"d":{"w":10,"h":10},"fx":[]},{"id":"b","type":"shape","name":"Bar","from":0,"dur":2,"on":true,"p":{"opacity":{"v":100,"kf":[]}},"d":{"w":10,"h":10},"fx":[]}]}"##).unwrap();
    kernel
        .open("view-film", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let identity = pm_surface::host_api::Identity {
        realm: "view-timeline".into(),
        generation: 1,
        project: "view-film".into(),
    };
    let backend = Arc::new(Mutex::new(
        data::Backend::new(kernel.clone(), identity.clone(), "timeline", false).unwrap(),
    ));
    let mut service = data::Data(backend.clone());
    let subscribed = |service: &mut data::Data| {
        service
            .query(&identity, &pm_surface::host_api::Query::Project, None, None)
            .unwrap()
    };
    let frame = |backend: &Arc<Mutex<data::Backend>>| {
        backend
            .lock()
            .unwrap()
            .timeline_frame(&json!({"first":0,"visible":20}))
            .unwrap()
    };
    let before = subscribed(&mut service);
    assert_eq!(frame(&backend)["mode"], "Layers");
    assert_eq!(frame(&backend)["window"]["total"], 2);
    assert_eq!(frame(&backend)["shyLayers"], true);
    // What the tracks' own model saves when a command changes the view.
    let save = |key: &str, value: pm_model::Value| {
        let state = kernel.state();
        let mut ui = state.ui.get("view-film").cloned().unwrap_or_default();
        let mut timeline = ui.timeline.as_object().cloned().unwrap_or_default();
        timeline.insert(key, value);
        ui.timeline = pm_model::Value::Object(timeline);
        kernel
            .state_transaction(StateTransaction {
                expected_revision: Some(state.revision),
                ops: vec![StateOp::Ui {
                    project: "view-film".into(),
                    next: ui,
                }],
            })
            .wait()
            .unwrap()
            .unwrap();
    };
    // Hide Shy Layers: the shy layer's row goes; the layer stays in the project.
    save("hideShy", pm_model::Value::Bool(true));
    let hidden = subscribed(&mut service);
    assert_eq!(hidden.revision, before.revision, "no document edit");
    assert_ne!(hidden.value, before.value, "the subscription moved");
    let shown = frame(&backend);
    assert_eq!(shown["hideShy"], true);
    assert_eq!(shown["window"]["total"], 1);
    assert_eq!(shown["window"]["rows"][0]["layer"], "b");
    // Timeline in Tracks: the header's mode follows.
    save("mode", pm_model::Value::from("tracks"));
    assert_ne!(subscribed(&mut service).value, hidden.value);
    assert_eq!(frame(&backend)["mode"], "Tracks");
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
    // Three edits in a row: none replaces another. Main stores layer
    // numbers, type badges and toolbar density the same way.
    let a = graph_of(
        "export function activate({ timeline }) {
            timeline.chrome({ rowHeight: 40 });
            timeline.chrome({ gutterWidth: 340 });
            timeline.chrome({ showLayerNumbers: false, showTypeBadges: false, toolbarDensity: 'normal' });
        }
        export const view = () => jsx('text', { children: 'A' });",
    );
    let b = graph_of(
        "export function activate({ timeline }) { timeline.chrome({ rowHeight: 30, surfaceOrder: 'reversed' }); }
        export const view = () => jsx('text', { children: 'B' });",
    );
    finish(panels.mount_extension("chrome-a", &a, None).unwrap());
    chrome_when(&actor, |c| {
        c["timeline"]
            == json!({"rowHeight":40,"gutterWidth":340,"showLayerNumbers":false,"showTypeBadges":false,"toolbarDensity":"normal"})
    });
    finish(panels.mount_extension("chrome-b", &b, None).unwrap());
    chrome_when(&actor, |c| {
        c["timeline"]
            == json!({"rowHeight":30,"gutterWidth":340,"surfaceOrder":"reversed","showLayerNumbers":false,"showTypeBadges":false,"toolbarDensity":"normal"})
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
    // main's normalizeExportDefaults fallbacks, exactly.
    assert_eq!(row(&publication, "project.format")["value"], "mp4");
    assert_eq!(row(&publication, "project.range")["value"], "work");
    assert_eq!(row(&publication, "project.motion-blur")["value"], true);
    assert_eq!(row(&publication, "project.audio")["value"], true);
    assert_eq!(row(&publication, "project.resolution")["value"], 1.0);
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

#[test]
fn a_contributed_layer_types_parameters_are_properties_rows() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    kernel.state_transaction(pm_kernel::state::StateTransaction {
        expected_revision: None,
        ops: vec![pm_kernel::state::StateOp::Register { registry:"layerTypes".into(), entry: pm_kernel::state::Registration {
            token:"synthetic-layer-token".into(), owner:Caller::native().writer,
            id:"synthetic.cursor".into(), definition: data::model(&json!({"id":"synthetic.cursor","label":"Synthetic Cursor","version":1,"width":128,"height":128,
                "params":[{"k":"size","label":"Cursor size","def":1,"min":0.1,"max":4,"step":0.01},{"k":"fill","label":"Fill","def":"#ffffff","type":"color"},{"k":"swap","label":"Swap fill / outline","def":false,"type":"toggle"},
                    {"k":"cursor","label":"Cursor type","def":0,"min":0,"max":10,"step":1,"options":[{"v":0,"label":"Arrow"},{"v":1,"label":"I-beam"}]},
                    {"k":"radius","label":"Radius","def":5,"defaultFrom":{"dimension":"width","factor":0.5}}],
                "renderer":{"kind":"fragment","fragment":"void main(){fragColor=vec4(1.0);}"}})).unwrap(),
        }}],
    }).wait().unwrap().unwrap();
    let project = pm_model::json::parse(r##"{"id":"synthetic","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"a","type":"extension","name":"Cursor","from":0,"dur":10,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[]}},"d":{"definition":"synthetic.cursor","version":1,"w":128,"h":128,"params":{"size":{"v":2,"kf":[]},"fill":{"v":"#112233","kf":[]},"swap":{"v":true,"kf":[]},"cursor":{"v":1,"kf":[]},"radius":{"v":12,"kf":[]}}},"fx":[]}]}"##).unwrap();
    kernel
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    select_layer(&kernel, "a");
    let identity = pm_surface::host_api::Identity {
        realm: "inspector".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "inspector", false).unwrap();
    backend.sync().unwrap();
    let snapshot = properties::Properties::default()
        .snapshot(backend.model.as_mut().unwrap(), 0., false)
        .unwrap();
    let rows: Vec<Value> = snapshot["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["rows"].as_array().unwrap().clone())
        .collect();
    let row = |id: &str| {
        rows.iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("{id} in {rows:?}"))
            .clone()
    };
    let size = row("content.x.size");
    assert_eq!(size["label"], "Cursor size");
    assert_eq!(size["control"]["axes"][0]["value"], 2.);
    assert_eq!(size["control"]["axes"][0]["min"], 0.1);
    assert_eq!(size["animation"], "static");
    assert_eq!(row("content.x.fill")["control"]["kind"], "colour");
    assert_eq!(row("content.x.fill")["control"]["value"], "#112233");
    assert_eq!(row("content.x.swap")["control"]["kind"], "toggle");
    assert_eq!(row("content.x.swap")["label"], "Swap fill / outline");
    // A numeric parameter with options is a choice, as on main.
    let cursor = row("content.x.cursor");
    assert_eq!(cursor["control"]["kind"], "choice");
    assert_eq!(
        cursor["control"]["options"][1],
        json!({"v":1,"label":"I-beam"})
    );
    // Reset returns a parameter to its definition's default (`defaultFrom`:
    // half the layer's 128 px width), and Undo brings the authored value back.
    let x = |key: &str| {
        let mut inspector = pm_panel_inspector::Inspector::new(
            kernel.published("synthetic").unwrap(),
            Default::default(),
        );
        inspector
            .binding_state(
                &pm_panel_inspector::Binding::Property(pm_panel_inspector::PropertyId::new(
                    "a",
                    format!("x.{key}"),
                )),
                0.,
            )
            .unwrap()
            .value
            .as_f64()
    };
    for (key, default) in [("radius", 64.), ("size", 1.)] {
        backend.sync().unwrap();
        let request = backend
            .model
            .as_mut()
            .unwrap()
            .property_action(
                Caller::native(),
                &[pm_panel_inspector::PropertyId::new("a", format!("x.{key}"))],
                pm_panel_inspector::PropertyAction::Reset(None),
                0.,
            )
            .unwrap();
        kernel.call("synthetic", request).wait().unwrap().unwrap();
        assert_eq!(x(key), Some(default), "{key} reset");
    }
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::HistoryCall::Undo),
        )
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(x("size"), Some(2.), "undo restores the authored size");
    // The mounted Properties panel draws the row, and its field writes the layer.
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    finish(panels.mount("inspector").unwrap());
    let publication = panels.take_publications().pop().unwrap();
    let ids: Vec<String> = publication
        .surface
        .nodes()
        .values()
        .filter_map(|n| n.props.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    let field = publication
        .surface
        .nodes()
        .values()
        .find(|n| n.props.get("id").and_then(Value::as_str) == Some("value:content.x.size:0"))
        .unwrap_or_else(|| panic!("size field in {ids:?}"));
    let handle = field.props["binding"].as_str().unwrap().to_owned();
    finish(panels.write("inspector", &handle, json!(3.)).unwrap());
    let mut inspector = pm_panel_inspector::Inspector::new(
        kernel.published("synthetic").unwrap(),
        Default::default(),
    );
    let size = inspector
        .binding_state(
            &pm_panel_inspector::Binding::Property(pm_panel_inspector::PropertyId::new(
                "a", "x.size",
            )),
            0.,
        )
        .unwrap();
    assert_eq!(size.value.as_f64(), Some(3.));
    panels.close();
}

#[test]
fn type_settings_list_a_variable_fonts_axes_and_saved_axes_of_a_missing_font() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let project = pm_model::json::parse(r##"{"id":"synthetic","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[
      {"id":"skia","type":"text","name":"Variable","from":0,"dur":10,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[]}},"d":{"text":"A","font":"Skia","color":"#112233"},"fx":[]},
      {"id":"plain","type":"text","name":"Static","from":0,"dur":10,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[]}},"d":{"text":"B","font":"Helvetica","color":"#112233"},"fx":[]},
      {"id":"gone","type":"text","name":"Missing","from":0,"dur":10,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[]}},"d":{"text":"C","font":"No Such Family 7f3a","color":"#112233","fontAxis.wdth":{"v":88.62,"kf":[],"expr":null}},"fx":[]}]}"##).unwrap();
    kernel
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let select = |id: &str| {
        kernel
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
    };
    let mut presenter = properties::Properties::default();
    let section = |id: &str, presenter: &mut properties::Properties| {
        select(id);
        let mut model = pm_panel_inspector::Inspector::new(
            kernel.published("synthetic").unwrap(),
            Default::default(),
        );
        let publication = presenter.snapshot(&mut model, 0., false).unwrap();
        let found = publication["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "type")
            .cloned();
        (found, model)
    };
    // A variable font: one row per axis, Weight among them.
    let (variable, mut model) = section("skia", &mut presenter);
    let variable = variable.expect("Skia has variation axes");
    let weight = variable["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["label"] == "Weight")
        .expect("Weight axis")
        .clone();
    assert!(variable.get("action").is_none());
    // Typing a value stores the axis the layer didn't have yet.
    let handle = weight["control"]["axes"][0]["binding"]["handle"]
        .as_str()
        .unwrap();
    let request = presenter
        .write(&mut model, Caller::native(), handle, 1.5f64.into(), 0.)
        .unwrap();
    let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
    assert!(!matches!(ack.reply, pm_kernel::Reply::Throw(_)));
    model.sync(kernel.published("synthetic").unwrap());
    let text = pm_model::json::to_canonical_string(&model.layer("skia").unwrap().to_owned());
    assert!(text.contains("fontAxis.wght"), "{text}");
    // A static font has nothing to set.
    assert!(section("plain", &mut presenter).0.is_none());
    // A missing font keeps the axes the layer saved, says so, offers a refresh.
    let gone = section("gone", &mut presenter)
        .0
        .expect("saved axes stay visible");
    let rows = gone["rows"].as_array().unwrap();
    assert!(
        rows.iter()
            .any(|r| r["label"] == "Width" && r["control"]["axes"][0]["value"] == 88.62)
    );
    assert!(
        rows.iter()
            .any(|r| r["control"]["kind"] == "status" && r["control"]["value"] == "Not installed")
    );
    assert_eq!(gone["action"], "refresh-fonts");
}

/// 3d-studio.md §8 through the Properties presenter: each role's sections,
/// a channel row writing like any property, and pop-ups and checkboxes
/// going through the Studio's typed edits.
#[test]
fn properties_show_the_3d_section_for_models_lights_and_cameras() {
    let actor = actor();
    let kernel = actor.handle();
    let run = |args: Value| {
        let call = pm_kernel::Call::Transact {
            label: None,
            expected_revision: None,
            body: pm_kernel::WriteBody::Commands(vec![
                pm_kernel::Scene3dCommand::Apply {
                    comp: None,
                    args: data::model(&args).unwrap(),
                    selection: vec![],
                    time: 0.,
                    autokey: false,
                }
                .into(),
            ]),
        };
        let ack = kernel
            .call("synthetic", Request::new(Caller::native(), call))
            .wait()
            .unwrap()
            .unwrap();
        if let pm_kernel::Reply::Throw(e) = &ack.reply {
            panic!("{args}: {e}");
        }
    };
    run(json!({"operation":"add_object","id":"model","object":{"source":{"primitive":"box"}}}));
    run(json!({"operation":"add_light","id":"lamp","light":{"type":"area"}}));
    run(json!({"operation":"add_camera","id":"cam"}));
    let mut presenter = properties::Properties::default();
    let publish = |presenter: &mut properties::Properties, id: &str| {
        select_layer(&kernel, id);
        let mut model = pm_panel_inspector::Inspector::new(
            kernel.published("synthetic").unwrap(),
            Default::default(),
        );
        let publication = presenter.snapshot(&mut model, 0., false).unwrap();
        (model, publication)
    };
    let titles = |p: &Value| {
        p["sections"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["id"].as_str().unwrap().starts_with("3d."))
            .map(|s| s["title"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let row = |p: &Value, id: &str| {
        p["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["rows"].as_array().unwrap())
            .find(|r| r["id"] == id)
            .cloned()
    };
    let labels = |p: &Value, section: &str| {
        p["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["title"] == section)
            .map(|s| {
                s["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["label"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let write = |presenter: &properties::Properties,
                 model: &mut pm_panel_inspector::Inspector,
                 row: &Value,
                 value: Value| {
        let handle = row["control"]["binding"]["handle"]
            .as_str()
            .or(row["control"]["axes"][0]["binding"]["handle"].as_str())
            .unwrap();
        let request = presenter
            .write(
                model,
                Caller::native(),
                handle,
                data::model(&value).unwrap(),
                0.,
            )
            .unwrap();
        let ack = kernel.call("synthetic", request).wait().unwrap().unwrap();
        if let pm_kernel::Reply::Throw(e) = &ack.reply {
            panic!("{value}: {e}");
        }
        model.sync(kernel.published("synthetic").unwrap());
    };

    let (mut model, p) = publish(&mut presenter, "model");
    assert_eq!(
        titles(&p),
        [
            "Model",
            "Material",
            "Coat and Glass",
            "Surface",
            "Emission",
            "Texture Maps",
            "Material Options"
        ]
    );
    // A 3D layer is named for what it is and has no 2D canvas rows.
    assert_eq!(p["target"]["first"]["typeLabel"], "3D Shape");
    assert_eq!(p["target"]["first"]["studio"], true);
    assert!(row(&p, "content.w").is_none());
    // The shadow switches only show while one is off (AE 27 Material Options
    // replace them).
    assert_eq!(labels(&p, "Model"), ["Source"]);
    assert_eq!(
        labels(&p, "Material"),
        [
            "Finish",
            "Screen",
            "Color",
            "Roughness",
            "Metalness",
            "Opacity"
        ]
    );
    assert_eq!(row(&p, "3d.finish").unwrap()["hidden"], true);
    let shelf = p["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["title"] == "Material")
        .unwrap()["finishes"]
        .clone();
    assert_eq!(shelf.as_array().unwrap().len(), 11);
    assert_eq!(
        labels(&p, "Material Options"),
        [
            "Casts Shadows",
            "Accepts Shadows",
            "Accepts Lights",
            "Appears in Reflections"
        ]
    );
    assert_eq!(row(&p, "3d.source").unwrap()["control"]["value"], "Cube");
    let roughness = row(&p, "m.roughness").unwrap();
    assert_eq!(roughness["control"]["axes"][0]["min"], 0.02);
    assert_eq!(roughness["animation"], "static");
    write(&presenter, &mut model, &roughness, json!(0.5));
    write(
        &presenter,
        &mut model,
        &row(&p, "3d.options.castsShadows").unwrap(),
        json!("only"),
    );
    write(
        &presenter,
        &mut model,
        &row(&p, "3d.finish").unwrap(),
        json!("gold"),
    );
    let (_, p) = publish(&mut presenter, "model");
    assert_eq!(
        row(&p, "3d.options.castsShadows").unwrap()["control"]["value"],
        "only"
    );
    // The finish wrote its own roughness over the earlier edit.
    assert_ne!(
        row(&p, "m.roughness").unwrap()["control"]["axes"][0]["value"],
        0.5
    );
    let transform = p["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "transform")
        .unwrap()
        .clone();
    assert_eq!(
        transform["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["control"]["axes"].as_array().unwrap().len())
            .collect::<Vec<_>>(),
        [3, 3, 3]
    );

    let (mut model, p) = publish(&mut presenter, "lamp");
    assert_eq!(titles(&p), ["Light"]);
    // Everything the Light tool's panel held is here, with the light: no
    // separate panel (3d-studio.md §8).
    assert_eq!(
        labels(&p, "Light"),
        [
            "Type",
            "Color",
            "Brightness",
            "Width",
            "Height",
            "Temperature",
            "Distance",
            "Cast Shadows"
        ]
    );
    // A light aims by its target: Transform shows it under Position, and
    // hides the rotation the renderer ignores.
    let transform: Vec<String> = p["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "transform")
        .unwrap()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["label"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(transform, ["Position", "Target"]);
    let stored = |id: &str| {
        let p = kernel.published("synthetic").unwrap();
        let root = data::wire(&p.snapshot.root().unwrap().to_value());
        fn find(v: &Value, id: &str) -> Option<Value> {
            match v {
                Value::Object(o)
                    if o.get("id").and_then(Value::as_str) == Some(id) && o.contains_key("p") =>
                {
                    Some(v.clone())
                }
                Value::Object(o) => o.values().find_map(|c| find(c, id)),
                Value::Array(a) => a.iter().find_map(|c| find(c, id)),
                _ => None,
            }
        }
        find(&root, id).unwrap()
    };
    // Temperature writes the light's colour; Distance slides it along its
    // aim, both as one Undo through the Studio's typed edits.
    let before = stored("lamp");
    write(
        &presenter,
        &mut model,
        &row(&p, "3d.light.adjust.temperature").unwrap(),
        json!(3200.),
    );
    let warm = stored("lamp");
    assert_ne!(
        warm["d"]["data"]["light"]["p"]["color"],
        before["d"]["data"]["light"]["p"]["color"]
    );
    let (mut model, p) = publish(&mut presenter, "lamp");
    let distance = row(&p, "3d.light.adjust.distance").unwrap();
    let was = distance["control"]["axes"][0]["value"].as_f64().unwrap();
    write(&presenter, &mut model, &distance, json!(was * 2.));
    let (mut model, p) = publish(&mut presenter, "lamp");
    let now = row(&p, "3d.light.adjust.distance").unwrap()["control"]["axes"][0]["value"]
        .as_f64()
        .unwrap();
    assert!((now - was * 2.).abs() < 1e-6, "{was} -> {now}");
    write(
        &presenter,
        &mut model,
        &row(&p, "3d.type").unwrap(),
        json!("spot"),
    );
    let (_, p) = publish(&mut presenter, "lamp");
    assert_eq!(
        labels(&p, "Light"),
        [
            "Type",
            "Color",
            "Brightness",
            "Range",
            "Falloff",
            "Spread",
            "Softness",
            "Temperature",
            "Distance",
            "Cast Shadows"
        ]
    );

    let (mut model, p) = publish(&mut presenter, "cam");
    assert_eq!(titles(&p), ["Camera", "Environment", "Shot"]);
    assert_eq!(
        labels(&p, "Camera"),
        [
            "Projection",
            "Field of View",
            "Clip Start",
            "Clip End",
            "f-Stop",
            "Focus"
        ]
    );
    assert_eq!(
        labels(&p, "Environment"),
        [
            "Ambient",
            "Ambient Color",
            "Exposure",
            "Shadows",
            "Background"
        ]
    );
    write(
        &presenter,
        &mut model,
        &row(&p, "3d.projection").unwrap(),
        json!("orthographic"),
    );
    let (_, p) = publish(&mut presenter, "cam");
    assert_eq!(labels(&p, "Camera")[1], "Zoom");
    // The camera shows the composition's Render Quality, not a second sampler.
    assert!(row(&p, "3d.samples").is_none());
    assert_eq!(
        row(&p, "3d.scene.quality").unwrap()["control"]["value"],
        "production"
    );
    // With nothing selected, the composition's 3D Scene section sets quality
    // and sky for the whole scene.
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
    let mut model = pm_panel_inspector::Inspector::new(
        kernel.published("synthetic").unwrap(),
        Default::default(),
    );
    let p = presenter.snapshot(&mut model, 0., false).unwrap();
    let scene = row(&p, "3d.scene.quality").unwrap();
    write(&presenter, &mut model, &scene, json!("final"));
    let p = presenter.snapshot(&mut model, 0., false).unwrap();
    assert_eq!(
        row(&p, "3d.scene.quality").unwrap()["control"]["value"],
        "final"
    );
    assert_eq!(row(&p, "3d.scene.sky").unwrap()["control"]["value"], "none");
    // A 2D shape has no 3D section, only Extrude, off.
    let (mut model, p) = publish(&mut presenter, "a");
    assert_eq!(titles(&p), ["Extrude"]);
    let on = row(&p, "3d.extrude.enabled").unwrap();
    assert_eq!(on["control"]["value"], false);
    // One click: depth from the artwork's size, a rounded bevel and four
    // surfaces, the composition already being lit.
    write(&presenter, &mut model, &on, json!(true));
    let p = presenter.snapshot(&mut model, 0., false).unwrap();
    assert_eq!(titles(&p), ["Extrude", "Front", "Bevel", "Side", "Back"]);
    assert_eq!(
        row(&p, "geo.depth").unwrap()["control"]["axes"][0]["value"],
        32.
    );
    assert_eq!(
        row(&p, "3d.extrude.bevelStyle").unwrap()["control"]["value"],
        "convex"
    );
    // The face is a stored channel now, keyframeable like any property.
    assert!(row(&p, "ms.front.roughness").is_some());
}

#[test]
fn powermove_account_rows_and_actions_reach_the_app_through_settings() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", false).unwrap();
    backend.native.facts = json!({"cloud":{"revision":4,"signedIn":true,"identity":"@maker",
        "email":"person@example.com","rememberInstalls":false}});
    let publication = backend.settings_snapshot().unwrap();
    let rows = publication["rows"].as_array().unwrap();
    let remember = rows.iter().find(|r| r["id"] == "account.remember").unwrap();
    // The switch acts through a lease, like every other Settings switch.
    assert!(remember["binding"].as_str().is_some_and(|b| !b.is_empty()));
    let revision = publication["revision"].clone();
    for (action, command) in [
        (
            json!({"type":"write","id":"account.remember","value":true}),
            "cloudRememberOn",
        ),
        (
            json!({"type":"row","id":"account.sign-in","action":"account.sign-out"}),
            "cloudSignOut",
        ),
        (
            json!({"type":"row","id":"account.delete","action":"account.delete"}),
            "cloudDeleteAccount",
        ),
    ] {
        backend.native.platform.clear();
        backend
            .native_command(
                "panel.settings",
                &json!({"revision":revision,"action":action}),
            )
            .unwrap();
        assert_eq!(
            backend.native.platform,
            [("command".to_owned(), json!({ "name": command }))]
        );
    }
    // A new account state is a new publication.
    backend.native.facts["cloud"]["revision"] = json!(5);
    assert_ne!(backend.settings_snapshot().unwrap()["revision"], revision);
    // A button pressed on the older publication still lands while the newest
    // offers that row the same action; a write or a changed action does not.
    let stale = |action: Value| json!({"revision":revision,"action":action});
    backend.native.platform.clear();
    backend
        .native_command(
            "panel.settings",
            &stale(json!({"type":"row","id":"account.sign-in","action":"account.sign-out"})),
        )
        .unwrap();
    assert_eq!(
        backend.native.platform,
        [("command".to_owned(), json!({ "name": "cloudSignOut" }))]
    );
    for action in [
        json!({"type":"write","id":"account.remember","value":false}),
        json!({"type":"row","id":"account.sign-in","action":"account.sign-in"}),
    ] {
        backend.native.platform.clear();
        assert!(
            backend
                .native_command("panel.settings", &stale(action))
                .is_err()
        );
        assert!(backend.native.platform.is_empty());
    }
}
/// Settings › General › Timeline › Hover to Scrub: the app's skimming choice
/// (View ▸ Skimming), on until turned off, shown with or without a project,
/// written through the app, and republished when the menu changes it.
#[test]
fn hover_to_scrub_is_a_settings_switch_that_follows_and_drives_skimming() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", false).unwrap();
    let skim = |p: &Value| {
        p["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "timeline.skimming")
            .cloned()
            .unwrap()
    };
    // Never chosen: on, as hovering has always scrubbed.
    let first = backend.settings_snapshot().unwrap();
    let row = skim(&first);
    assert_eq!(row["control"], "switch");
    assert_eq!(row["section"], "general");
    assert_eq!(row["label"], "Hover to Scrub");
    assert_eq!(row["value"], true);
    assert!(row["binding"].as_str().is_some_and(|b| !b.is_empty()));
    // Turned off in Settings: the app is told, and the row shows it at once.
    backend.native.platform.clear();
    let models::NativeCommand::Value(after) = backend
        .native_command(
            "panel.settings",
            &json!({"revision":first["revision"],"action":{"type":"write","id":"timeline.skimming","value":false}}),
        )
        .unwrap()
    else {
        panic!("the new publication")
    };
    assert_eq!(
        backend.native.platform,
        [("command".to_owned(), json!({ "name": "skimmingOff" }))]
    );
    assert_eq!(skim(&after)["value"], false);
    // Only on or off.
    let latest = backend.settings_snapshot().unwrap();
    assert!(
        backend
            .native_command(
                "panel.settings",
                &json!({"revision":latest["revision"],"action":{"type":"write","id":"timeline.skimming","value":"yes"}}),
            )
            .is_err()
    );
    // View ▸ Skimming turned it back on: a new publication shows it.
    backend.native.facts["skimming"] = json!(true);
    let menu = backend.settings_snapshot().unwrap();
    assert_ne!(menu["revision"], latest["revision"]);
    assert_eq!(skim(&menu)["value"], true);
}
/// A press from an older Settings publication acts only on the account or
/// provider that publication showed: provider B replacing A, or another
/// account signing in, refuses a stale Disconnect / Sign Out / Delete.
#[test]
fn stale_settings_presses_never_reach_a_different_account_or_provider() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", false).unwrap();
    let provider = |revision: u64, url: &str| {
        json!({"revision":revision,"connected":true,"type":"OpenAI-compatible","url":url,
            "model":"m","hasKey":true})
    };
    backend.native.facts = json!({
        "cloud":{"revision":1,"signedIn":true,"identity":"@maker","email":"a@example.com"},
        "provider":provider(1, "http://a.invalid/v1"),
    });
    let seen = backend.settings_snapshot().unwrap()["revision"].clone();
    assert!(
        backend.settings_snapshot().unwrap()["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row.get("target").is_none()),
        "targets are not published"
    );
    let press = |backend: &mut data::Backend, revision: &Value, id: &str, action: &str| {
        backend.native.platform.clear();
        let landed = backend
            .native_command(
                "panel.settings",
                &json!({"revision":revision,"action":{"type":"row","id":id,"action":action}}),
            )
            .is_ok();
        landed && !backend.native.platform.is_empty()
    };
    // An unrelated change (the same provider rechecked) keeps a stale press.
    backend.native.facts["provider"] = provider(2, "http://a.invalid/v1");
    assert!(press(
        &mut backend,
        &seen,
        "account.disconnect",
        "provider.disconnect"
    ));
    // Provider B replaced A: Disconnect seen on A must not disconnect B.
    let seen = backend.settings_snapshot().unwrap()["revision"].clone();
    backend.native.facts["provider"] = provider(3, "http://b.invalid/v1");
    assert!(!press(
        &mut backend,
        &seen,
        "account.disconnect",
        "provider.disconnect"
    ));
    // Another Powermove account signed in: Sign Out and Delete seen on the
    // first refuse; the same account, rechecked, still signs out.
    let seen = backend.settings_snapshot().unwrap()["revision"].clone();
    backend.native.facts["cloud"] =
        json!({"revision":2,"signedIn":true,"identity":"@other","email":"b@example.com"});
    assert!(!press(
        &mut backend,
        &seen,
        "account.sign-in",
        "account.sign-out"
    ));
    assert!(!press(
        &mut backend,
        &seen,
        "account.delete",
        "account.delete"
    ));
    let seen = backend.settings_snapshot().unwrap()["revision"].clone();
    backend.native.facts["cloud"]["revision"] = json!(3);
    assert!(press(
        &mut backend,
        &seen,
        "account.sign-in",
        "account.sign-out"
    ));
    // A publication no longer remembered never carries a destructive press;
    // a harmless one (Sign In) still lands while offered.
    assert!(!press(
        &mut backend,
        &json!(0),
        "account.sign-in",
        "account.sign-out"
    ));
    backend.native.facts["cloud"] = json!({"revision":4,"signedIn":false});
    assert!(press(
        &mut backend,
        &json!(0),
        "account.sign-in",
        "account.sign-in"
    ));
}
/// Settings › Media's proxy choices show their pop-ups (a choice without a
/// lease drew no control at all) and a change reaches the app's media service.
#[test]
fn media_proxy_choices_have_controls_and_write_through_the_app() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", false).unwrap();
    backend.native.facts = json!({"mediaSettings":{"rows":[
        {"id":"media.create","label":"Create Proxies","detail":"","value":"ask",
         "options":[{"v":"ask","label":"When Asked"},{"v":"import","label":"On Import"}]}],
        "location":"/synthetic","used":null,"folders":[]}});
    let publication = backend.settings_snapshot().unwrap();
    let row = publication["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "media.create")
        .unwrap()
        .clone();
    assert_eq!(row["control"], "choice");
    assert!(row["binding"].as_str().is_some_and(|b| !b.is_empty()));
    backend.native.platform.clear();
    backend
        .native_command(
            "panel.settings",
            &json!({"revision":publication["revision"],
                "action":{"type":"write","id":"media.create","value":"import"}}),
        )
        .unwrap();
    assert_eq!(
        backend.native.platform,
        [(
            "media.settings".to_owned(),
            json!({"write":"media.create","value":"import"})
        )]
    );
}
/// This Mac's settings (Settings › Media, General › Playback) show with no
/// project open (a window on Home or a fresh launch), and a change to them
/// made elsewhere republishes Settings.
#[test]
fn media_and_playback_rows_show_without_a_project_and_follow_their_fact() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "builtin-application".into(),
    };
    // The application realm: Settings with no project (Home, a fresh launch).
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", true).unwrap();
    let fact = |limit: u32| {
        json!({"mediaSettings":{"rows":[
            {"id":"media.renderLimit","label":"Render Files Limit","detail":"","value":limit,
             "options":[{"v":10,"label":"10 GB"},{"v":25,"label":"25 GB"}]},
            {"id":"media.backgroundRender","label":"Background Rendering","detail":"","value":true,
             "options":[{"v":true,"label":"On"},{"v":false,"label":"Off"}],
             "section":"general","group":"Playback","control":"switch"}],
            "location":"/synthetic","used":null,"folders":[]}})
    };
    backend.native.facts = fact(25);
    let publication = backend.settings_snapshot().unwrap();
    assert!(publication.get("project").is_none());
    let rows = publication["rows"].as_array().unwrap();
    let row = |id: &str| rows.iter().find(|r| r["id"] == id).cloned().unwrap();
    assert_eq!(row("media.renderLimit")["section"], "media");
    let playback = row("media.backgroundRender");
    assert_eq!(
        (
            &playback["section"],
            &playback["group"],
            &playback["control"]
        ),
        (&json!("general"), &json!("Playback"), &json!("switch"))
    );
    let before = backend.settings_snapshot().unwrap()["revision"].clone();
    backend.native.facts = fact(10);
    assert_ne!(backend.settings_snapshot().unwrap()["revision"], before);
}
#[test]
fn storage_rows_show_without_a_project_and_a_new_measurement_republishes() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "builtin-application".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", true).unwrap();
    let fact = |value: &str| {
        json!({"storageSettings":{"rows":[
            {"id":"storage.native.total","section":"storage","group":"Powermove",
             "label":"Space Used","control":"status","value":value},
            {"id":"storage.original.backups","section":"storage","group":"Original App",
             "label":"Backups","control":"action","action":"storage.delete.backups",
             "actionLabel":"Delete…","value":"9.3 GB"}]}})
    };
    backend.native.facts = fact("Calculating…");
    let before = backend.settings_snapshot().unwrap();
    let rows = before["rows"].as_array().unwrap();
    assert!(rows.iter().any(|r| r["id"] == "storage.original.backups"));
    backend.native.facts = fact("1.2 GB");
    let after = backend.settings_snapshot().unwrap();
    assert_ne!(after["revision"], before["revision"]);
    // A removal is a destructive row action: it reaches the app as itself.
    assert!(super::settings::destructive_for_test(
        "storage.delete.backups"
    ));
}
#[test]
fn mods_show_in_finder_and_delete_reach_the_app_only_for_what_this_mac_installed() {
    let actor = actor();
    let entry = |id: &str, installed: bool| {
        pm_kernel::state::StateOp::Extension(pm_kernel::state::ExtensionEntry {
            id: id.into(),
            installed,
            enabled: true,
            version: "1.0.0".into(),
            source: pm_model::json::parse(
                r#"{"hostRecord":{"name":"Glow","health":{"state":"ok"}}}"#,
            )
            .unwrap(),
        })
    };
    actor
        .handle()
        .state_transaction(pm_kernel::state::StateTransaction {
            expected_revision: None,
            ops: vec![
                entry("com.example.glow", true),
                entry("builtin.panel", false),
                pm_kernel::state::StateOp::Extension(pm_kernel::state::ExtensionEntry {
                    id: "com.example.gone".into(),
                    installed: true,
                    enabled: true,
                    version: "1.0.0".into(),
                    source: pm_model::json::parse(
                        r#"{"hostRecord":{"name":"Gone","archived":true}}"#,
                    )
                    .unwrap(),
                }),
                pm_kernel::state::StateOp::Extension(pm_kernel::state::ExtensionEntry {
                    id: "com.example.away".into(),
                    installed: true,
                    enabled: true,
                    version: "1.0.0".into(),
                    source: pm_model::json::parse(
                        r#"{"hostRecord":{"name":"Away","about":"Glows","unavailable":true}}"#,
                    )
                    .unwrap(),
                }),
            ],
        })
        .wait()
        .unwrap()
        .unwrap();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", false).unwrap();
    let publication = backend.settings_snapshot().unwrap();
    let listed = |id: &str| {
        publication["extensions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == id)
            .unwrap()
            .clone()
    };
    // Deleted ones stay in the kernel (their values) but leave the list.
    assert!(
        !publication["extensions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == "com.example.gone")
    );
    assert_eq!(listed("com.example.glow")["canArchive"], true);
    assert_eq!(listed("builtin.panel")["canArchive"], false);
    // Its folder can't be reached (an unmounted volume): listed, and said so.
    assert_eq!(
        listed("com.example.away")["about"],
        "Unavailable: its folder can't be reached."
    );
    let revision = publication["revision"].clone();
    let mut perform = |action: Value| {
        backend.native.platform.clear();
        let result = backend.native_command(
            "panel.settings",
            &json!({"revision":revision,"action":action}),
        );
        (result.is_ok(), backend.native.platform.clone())
    };
    for (kind, command) in [
        ("extension.finder", "extensionReveal.com.example.glow"),
        ("extension.archive", "extensionArchive.com.example.glow"),
    ] {
        let (ok, platform) = perform(json!({"type":kind,"id":"com.example.glow"}));
        assert!(ok);
        assert_eq!(
            platform,
            [("command".to_owned(), json!({ "name": command }))]
        );
        // A built-in has no folder to show and is never deleted.
        let (_, platform) = perform(json!({"type":kind,"id":"builtin.panel"}));
        assert!(platform.is_empty());
    }
}

/// The Color panel, mounted as the app mounts it: it lists the layer's grade
/// (Color Wheels, and an extension's effect that says `grade: true`) and not
/// its other effects, opens every grading row, and a value set through one of
/// its controls is written to the document as Effect Controls writes it.
#[test]
fn the_color_panel_lists_the_grade_and_writes_through_its_controls() {
    let actor = actor();
    let kernel = actor.handle();
    // An extension's grading effect.
    kernel.state_transaction(pm_kernel::state::StateTransaction {
        expected_revision: None,
        ops: vec![pm_kernel::state::StateOp::Register { registry:"effects".into(), entry: pm_kernel::state::Registration {
            token:"synthetic-look-token".into(), owner:Caller::native().writer,
            id:"synthetic.look".into(), definition: data::model(&json!({"id":"synthetic.look","label":"Synthetic Look","version":1,"grade":true,
                "frag":"void main(){fragColor=texture(u_tex,v_st);}",
                "params":[{"k":"strength","label":"Strength","type":"number","def":0.5,"step":0.01}]})).unwrap(),
        }}],
    }).wait().unwrap().unwrap();
    select_layer(&kernel, "a");
    let mut setup = data::Backend::new(
        kernel.clone(),
        pm_surface::host_api::Identity {
            realm: "color-setup".into(),
            generation: 1,
            project: "synthetic".into(),
        },
        "layer-effects",
        false,
    )
    .unwrap();
    for effect in ["colorWheels", "blur", "synthetic.look", "applyColorLut"] {
        setup.sync().unwrap();
        let request = setup
            .model
            .as_mut()
            .unwrap()
            .add_effect_at(Caller::native(), "a", effect, 0.)
            .unwrap();
        kernel.call("synthetic", request).wait().unwrap().unwrap();
    }
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    panels.update_facts(json!({"time":0,"playing":false,"stage":[640.,360.]}));
    finish(panels.mount("color").unwrap());
    let backend = panels
        .broker
        .entry("color")
        .unwrap()
        .lock()
        .unwrap()
        .backend
        .clone();
    // The panel asks for every row; wait for it to arrive.
    let deadline = Instant::now() + Duration::from_secs(20);
    let projection = loop {
        let p = backend.lock().unwrap().projection().unwrap();
        if p["range"] == json!([0, p["count"]]) && p["count"].as_f64() > Some(20.) {
            break p;
        }
        assert!(
            Instant::now() < deadline,
            "the panel never asked for its rows: {p}"
        );
        thread::sleep(Duration::from_millis(5));
    };
    let headers: Vec<_> = projection["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "effect")
        .map(|r| (r["definition"].clone(), r["grade"].clone()))
        .collect();
    assert_eq!(
        headers,
        [
            (json!("colorWheels"), json!(true)),
            (json!("blur"), json!(false)),
            (json!("synthetic.look"), json!(true)),
            (json!("applyColorLut"), json!(true)),
        ]
    );
    // What the person sees: the grade's controls and not Blur's.
    let publication = panels.take_publications().pop().unwrap();
    let labels: Vec<String> = publication
        .surface
        .nodes()
        .values()
        .filter_map(|n| {
            n.props
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    // Every grade in the order it applies, each under its own name (an
    // extension's included), and the panel's header actions: Add Grade (a
    // menu of Color Wheels, Curve, Color Space Transform, LUT…, CDL…) and
    // CDL (Import, Export).
    for label in [
        "Amount",
        "Input Space",
        "Color Wheels",
        "Lift",
        "Temperature",
        "Add Grade",
        "CDL",
        "Synthetic Look",
        "Strength",
        "Remove Color Wheels",
    ] {
        assert!(
            labels.iter().any(|l| l == label),
            "{label} is not shown: {labels:?}"
        );
    }
    assert!(
        !labels.iter().any(|l| l == "Blurriness"),
        "Blur is not a grade"
    );
    // A value set through the panel's Gain Master control.
    let gain = projection["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "parameter" && r["parameter"]["key"] == "gainMaster")
        .unwrap()["parameter"]
        .clone();
    finish(
        panels
            .write("color", gain["binding"].as_str().unwrap(), json!(1.4))
            .unwrap(),
    );
    let p = kernel.published("synthetic").unwrap();
    let mut model = pm_panel_inspector::Inspector::new(p, Default::default());
    let id = pm_panel_inspector::PropertyId::from_value(
        &pm_model::json::parse(gain["id"].as_str().unwrap()).unwrap(),
    )
    .unwrap();
    let state = model
        .binding_state(&pm_panel_inspector::Binding::Property(id), 0.)
        .unwrap();
    assert_eq!(state.value.as_f64(), Some(1.4));
}

/// A colour wheel, as a person drags it: the Color panel draws the wheel with
/// one binding over its red, green and blue, and a drag writes all three in
/// one gesture: the grade changes as one Undo step, the luma kept by the
/// wheel's own values (the binding refuses a wrong length or a foreign row).
#[test]
fn a_color_wheel_drag_writes_red_green_and_blue_as_one_undo_step() {
    let actor = actor();
    let kernel = actor.handle();
    select_layer(&kernel, "a");
    let mut setup = data::Backend::new(
        kernel.clone(),
        pm_surface::host_api::Identity {
            realm: "wheel-setup".into(),
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
        .add_effect_at(Caller::native(), "a", "colorWheels", 0.)
        .unwrap();
    kernel.call("synthetic", request).wait().unwrap().unwrap();
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    panels.update_facts(json!({"time":0,"playing":false,"stage":[640.,360.]}));
    finish(panels.mount("color").unwrap());
    // The Lift wheel the panel draws, and the binding it drags through.
    let deadline = Instant::now() + Duration::from_secs(20);
    let binding = loop {
        let found = panels.take_publications().into_iter().rev().find_map(|p| {
            p.surface.nodes().values().find_map(|n| {
                (n.props.get("data-pm-component") == Some(&json!("color-wheel"))
                    && n.props.get("label") == Some(&json!("Lift colour")))
                .then(|| n.props["binding"].as_str().unwrap().to_owned())
            })
        });
        if let Some(b) = found {
            break b;
        }
        assert!(
            Instant::now() < deadline,
            "the panel never drew the Lift wheel"
        );
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(
        binding.matches('+').count(),
        2,
        "red, green and blue: {binding}"
    );
    let before = kernel.published("synthetic").unwrap().history.entries;
    let ack = |pending: Result<
        pm_kernel::Pending<std::result::Result<pm_kernel::Ack, pm_kernel::KernelFailure>>,
    >| pending.unwrap().wait().unwrap().unwrap();
    let gesture = match ack(panels.scrub_begin("color")).reply {
        pm_kernel::Reply::Gesture(id) => id,
        other => panic!("no gesture: {other:?}"),
    };
    for value in [json!([0.05, -0.02, 0.1]), json!([0.1, -0.04, 0.2])] {
        ack(panels.scrub_step("color", &binding, gesture, value));
    }
    // A step of the wrong length is refused; the drag goes on.
    assert!(
        panels
            .scrub_step("color", &binding, gesture, json!([0.3]))
            .is_err()
    );
    ack(panels.scrub_finish("color", gesture, true));
    let p = kernel.published("synthetic").unwrap();
    assert_eq!(p.history.entries, before + 1, "one Undo step");
    let backend = panels
        .broker
        .entry("color")
        .unwrap()
        .lock()
        .unwrap()
        .backend
        .clone();
    let projection = backend.lock().unwrap().projection().unwrap();
    let mut model = pm_panel_inspector::Inspector::new(p, Default::default());
    for (key, want) in [("liftRed", 0.1), ("liftGreen", -0.04), ("liftBlue", 0.2)] {
        let id = projection["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "parameter" && r["parameter"]["key"] == key)
            .unwrap()["parameter"]["id"]
            .clone();
        let id = pm_panel_inspector::PropertyId::from_value(
            &pm_model::json::parse(id.as_str().unwrap()).unwrap(),
        )
        .unwrap();
        let state = model
            .binding_state(&pm_panel_inspector::Binding::Property(id), 0.)
            .unwrap();
        assert_eq!(state.value.as_f64(), Some(want), "{key}");
    }
    // A group naming rows of two effects, or out of order, is no binding.
    let rows: Vec<&Value> = projection["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "parameter")
        .collect();
    let pick = |key: &str| {
        rows.iter()
            .find(|r| r["parameter"]["key"] == key)
            .map(|r| r["parameter"].clone())
            .unwrap()
    };
    let (r, g) = (pick("liftRed"), pick("liftGreen"));
    let handle = format!(
        "{}+{}",
        r["binding"].as_str().unwrap(),
        g["binding"].as_str().unwrap()
    );
    let ids = |list: &[&Value]| {
        list.iter()
            .map(|v| v["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert!(data::effect_binding_rows(&projection, &handle, &ids(&[&r, &g])).is_some());
    assert!(data::effect_binding_rows(&projection, &handle, &ids(&[&g, &r])).is_none());
    assert!(data::effect_binding_rows(&projection, &handle, &ids(&[&r, &r])).is_none());
}

/// Two Color Wheels on one layer both grade the picture, so the Color panel
/// shows each with its own controls, switch and Reset (it showed the first
/// and hid the second everywhere).
#[test]
fn every_color_wheels_on_the_layer_has_its_controls_in_the_color_panel() {
    let actor = actor();
    let kernel = actor.handle();
    select_layer(&kernel, "a");
    let mut setup = data::Backend::new(
        kernel.clone(),
        pm_surface::host_api::Identity {
            realm: "color-setup".into(),
            generation: 1,
            project: "synthetic".into(),
        },
        "layer-effects",
        false,
    )
    .unwrap();
    for _ in 0..2 {
        setup.sync().unwrap();
        let request = setup
            .model
            .as_mut()
            .unwrap()
            .add_effect_at(Caller::native(), "a", "colorWheels", 0.)
            .unwrap();
        kernel.call("synthetic", request).wait().unwrap().unwrap();
    }
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    panels.update_facts(json!({"time":0,"playing":false,"stage":[640.,360.]}));
    finish(panels.mount("color").unwrap());
    let backend = panels
        .broker
        .entry("color")
        .unwrap()
        .lock()
        .unwrap()
        .backend
        .clone();
    let deadline = Instant::now() + Duration::from_secs(20);
    let projection = loop {
        let p = backend.lock().unwrap().projection().unwrap();
        if p["range"] == json!([0, p["count"]]) && p["count"].as_f64() > Some(40.) {
            break p;
        }
        assert!(
            Instant::now() < deadline,
            "the panel never asked for its rows: {p}"
        );
        thread::sleep(Duration::from_millis(5));
    };
    let rows = projection["rows"].as_array().unwrap();
    let wheels: Vec<&Value> = rows
        .iter()
        .filter(|r| r["kind"] == "effect" && r["definition"] == "colorWheels")
        .collect();
    assert_eq!(wheels.len(), 2);
    // Wait for the publication that draws every row the panel asked for.
    let ids = |publication: &Publication| -> Vec<String> {
        publication
            .surface
            .nodes()
            .values()
            .filter_map(|n| n.props.get("id").and_then(Value::as_str).map(str::to_owned))
            .collect()
    };
    let mut shown = Vec::new();
    while Instant::now() < deadline {
        if let Some(publication) = panels.take_publications().pop() {
            shown = ids(&publication);
        }
        if shown.iter().any(|id| id.starts_with("color:field:")) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    for header in &wheels {
        let effect = header["id"].as_str().unwrap();
        assert!(
            shown.contains(&format!("color:reset:{effect}")),
            "Color Wheels {effect} has no Reset: {shown:?}"
        );
        // A wheel's red, green and blue are its drawn wheel (one control
        // over the three); Tone and each Master are fields.
        assert!(
            shown.contains(&format!("color:wheel:{effect}:gain")),
            "the Gain wheel of Color Wheels {effect} is not drawn"
        );
        for key in ["exposure", "liftMaster", "gainMaster"] {
            let row = rows
                .iter()
                .find(|r| {
                    r["kind"] == "parameter"
                        && r["parameter"]["effect"] == effect
                        && r["parameter"]["key"] == key
                })
                .unwrap_or_else(|| panic!("{effect} has no {key} row"));
            let field = format!("color:field:{}", row["id"].as_str().unwrap());
            assert!(
                shown.contains(&field),
                "{key} of Color Wheels {effect} is not shown"
            );
        }
    }
}

/// Add Curve on a chosen channel: the Color panel's add intent carries the
/// new effect's first values, so a Green curve is made on Green in one Undo
/// step; a key the definition does not have, or a value outside its limits,
/// is refused and changes nothing.
#[test]
fn a_curve_is_added_on_its_channel_in_one_undo_step() {
    let actor = actor();
    let kernel = actor.handle();
    select_layer(&kernel, "a");
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    panels.update_facts(json!({"time":0,"playing":false,"stage":[640.,360.]}));
    finish(panels.mount("color").unwrap());
    let backend = panels
        .broker
        .entry("color")
        .unwrap()
        .lock()
        .unwrap()
        .backend
        .clone();
    let send = |intent: Value| {
        let projection = backend.lock().unwrap().projection().unwrap();
        let args = json!({"documentId":projection["documentId"],"revision":projection["revision"],
            "layer":"a","intent":intent});
        let ticket = panels
            .realms
            .lock()
            .unwrap()
            .get("color")
            .unwrap()
            .eval(format!(
                "__pmBuiltin.command('panel.effect-controls',{args})"
            ))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(result) = ticket.try_take() {
                return result;
            }
            assert!(Instant::now() < deadline, "v4 ticket timed out");
            thread::sleep(Duration::from_millis(1));
        }
    };
    let fx = || {
        let p = kernel.published("synthetic").unwrap();
        let model = pm_panel_inspector::Inspector::new(p, Default::default());
        crate::data::wire(&model.layer("a").unwrap())["fx"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while backend.lock().unwrap().projection().unwrap()["target"] != "Layer" {
        assert!(Instant::now() < deadline, "the panel never saw the layer");
        thread::sleep(Duration::from_millis(5));
    }
    let before = kernel.published("synthetic").unwrap().history.entries;
    let result = send(json!({"type":"add","definition":"colorCurves","parameters":{"channel":2}}));
    assert!(result.is_ok(), "{result:?}");
    let deadline = Instant::now() + Duration::from_secs(20);
    while fx().is_empty() {
        assert!(Instant::now() < deadline, "the curve was never added");
        for (_, ticket) in panels.publish() {
            finish(ticket.unwrap());
        }
    }
    let after = kernel.published("synthetic").unwrap();
    assert_eq!(after.history.entries, before + 1, "one Undo step");
    let added = fx();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["type"], "colorCurves");
    assert_eq!(added[0]["p"]["channel"]["v"], 2);
    for refused in [
        json!({"channel":1,"nope":3}),
        json!({"channel":9}),
        json!({"channel":{"v":1,"kf":[],"expr":null}}),
    ] {
        let _ = send(json!({"type":"add","definition":"colorCurves","parameters":refused}));
    }
    // A later valid add lands after the refused ones; they changed nothing.
    send(json!({"type":"add","definition":"colorCurves","parameters":{"channel":3}})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while fx().len() < 2 {
        assert!(
            Instant::now() < deadline,
            "the second curve was never added"
        );
        for (_, ticket) in panels.publish() {
            finish(ticket.unwrap());
        }
    }
    let all = fx();
    assert_eq!(all.len(), 2, "a refused add changes nothing: {all:?}");
    assert_eq!(all[1]["p"]["channel"]["v"], 3);
}

/// Settings › Shortcuts: the app's page reaches the publication with its
/// preset row, and every action on it goes to the app, which owns the keymap.
#[test]
fn the_shortcuts_page_publishes_the_apps_page_and_hands_its_actions_to_the_app() {
    let actor = actor();
    let identity = pm_surface::host_api::Identity {
        realm: "settings".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(actor.handle(), identity, "settings", false).unwrap();
    backend.native.facts = json!({"shortcuts":{"preset":"after-effects","changed":false,"stamp":1,
        "presets":[{"v":"after-effects","label":"After Effects"},{"v":"premiere-pro","label":"Premiere Pro"}],
        "rows":[{"id":"rippleTrimIn","menu":"Layer","label":"Ripple Trim In Point to Playhead",
            "keys":["⌥⇧["],"chords":["alt+shift+["],"changed":false,"fixed":false,"recording":false}],
        "conflicts":[]}});
    let publication = backend.settings_snapshot().unwrap();
    assert_eq!(publication["shortcuts"]["rows"][0]["id"], "rippleTrimIn");
    let row = |id: &str| {
        publication["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("no {id} row"))
    };
    assert_eq!(row("shortcuts.preset")["control"], "choice");
    assert_eq!(row("shortcuts.preset")["value"], "after-effects");
    assert!(row("shortcuts.preset")["binding"].is_string());
    assert_eq!(
        row("shortcuts.resetAll")["disabled"],
        true,
        "nothing to reset"
    );
    let revision = publication["revision"].clone();
    for (action, op) in [
        (
            json!({"type":"write","id":"shortcuts.preset","value":"premiere-pro"}),
            "preset",
        ),
        (
            json!({"type":"row","id":"shortcuts.import","action":"shortcuts.import"}),
            "import",
        ),
        (
            json!({"type":"row","id":"shortcuts.export","action":"shortcuts.export"}),
            "export",
        ),
        (
            json!({"type":"shortcut","op":"record","id":"rippleTrimIn"}),
            "record",
        ),
    ] {
        backend.native.platform.clear();
        backend
            .native_command(
                "panel.settings",
                &json!({"revision":revision,"action":action}),
            )
            .unwrap_or_else(|e| panic!("{action}: {e}"));
        let sent = backend
            .native
            .platform
            .iter()
            .find(|(kind, r)| kind == "command" && r["name"] == "shortcuts")
            .map(|(_, r)| r["args"][0]["op"].clone());
        assert_eq!(sent, Some(json!(op)), "{action}");
    }
    // A click from an older publication still changes a listed command.
    backend.native.platform.clear();
    backend
        .native_command(
            "panel.settings",
            &json!({"revision":0,"action":{"type":"shortcut","op":"record","id":"rippleTrimIn"}}),
        )
        .expect("a stale click on a listed command applies");
    assert!(
        backend
            .native_command(
                "panel.settings",
                &json!({"revision":0,"action":{"type":"shortcut","op":"record","id":"save"}}),
            )
            .is_err(),
        "a command the page does not list is refused"
    );
    // So does a preset chosen on an older publication.
    backend.native.platform.clear();
    backend
        .native_command(
            "panel.settings",
            &json!({"revision":0,"action":{"type":"write","id":"shortcuts.preset","value":"premiere-pro"}}),
        )
        .expect("a stale preset choice applies");
    // A preset the page does not offer is refused here.
    assert!(
        backend
            .native_command(
                "panel.settings",
                &json!({"revision":revision,"action":{"type":"write","id":"shortcuts.preset","value":"avid"}}),
            )
            .is_err()
    );
}

/// Round 7: an imported model's material slots are Properties sections with
/// the file's own values; an edit becomes a keyframeable `mi.<slot>.<key>`.
#[test]
fn properties_list_an_imported_models_parts_with_their_own_values() {
    let actor = actor();
    let kernel = actor.handle();
    let run = |args: Value| {
        let call = pm_kernel::Call::Transact {
            label: None,
            expected_revision: None,
            body: pm_kernel::WriteBody::Commands(vec![
                pm_kernel::Scene3dCommand::Apply {
                    comp: None,
                    args: data::model(&args).unwrap(),
                    selection: vec![],
                    time: 0.,
                    autokey: false,
                }
                .into(),
            ]),
        };
        let ack = kernel
            .call("synthetic", Request::new(Caller::native(), call))
            .wait()
            .unwrap()
            .unwrap();
        if let pm_kernel::Reply::Throw(e) = &ack.reply {
            panic!("{args}: {e}");
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Product.gltf");
    std::fs::write(
        &file,
        r#"{"asset":{"version":"2.0"},"materials":[{"name":"Aluminium","pbrMetallicRoughness":{"metallicFactor":1,"roughnessFactor":0.4}},{"name":"Glass"}]}"#,
    )
    .unwrap();
    run(
        json!({"operation":"add_object","id":"model","object":{"source":{"assetId":"a1"},"useSourceMaterials":true}}),
    );
    let mut presenter = properties::Properties::default();
    presenter.originals.insert("a1".into(), file);
    let publish = |presenter: &mut properties::Properties| {
        select_layer(&kernel, "model");
        let mut model = pm_panel_inspector::Inspector::new(
            kernel.published("synthetic").unwrap(),
            Default::default(),
        );
        presenter.snapshot(&mut model, 0., false).unwrap()
    };
    let row = |p: &Value, id: &str| {
        p["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["rows"].as_array().unwrap())
            .find(|r| r["id"] == id)
            .cloned()
    };
    let p = publish(&mut presenter);
    let titles: Vec<_> = p["sections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["id"].as_str().unwrap().starts_with("3d.part."))
        .map(|s| s["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(titles, ["Aluminium", "Glass"]);
    let rough = row(&p, "3d.part.0.roughness").expect("the file's roughness");
    assert_eq!(rough["control"]["axes"][0]["value"], 0.4);
    assert!(
        row(&p, "3d.part.1.map.color").is_some(),
        "a layer can be the part's colour"
    );
    run(
        json!({"operation":"set_material_properties","target":"model","materialIndex":0,"patch":{"roughness":0.1}}),
    );
    let p = publish(&mut presenter);
    let rough = row(&p, "mi.0.roughness").expect("an override is a channel row");
    assert_eq!(rough["control"]["axes"][0]["value"], 0.1);
    assert_eq!(rough["animation"], "static");
    assert!(row(&p, "3d.part.0.roughness").is_none());
    // A viewer double-click on the glass part opens and marks its section;
    // the same request doesn't reopen it after the user closes it.
    let section = |p: &Value, id: &str| {
        p["sections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(section(&p, "3d.part.1")["collapsed"], true);
    presenter.focus_part(&json!({"layer":"model","slot":1,"seq":1}));
    let p = publish(&mut presenter);
    assert_eq!(section(&p, "3d.part.1")["collapsed"], false);
    assert_eq!(section(&p, "3d.part.1")["focused"], true);
    assert_eq!(section(&p, "3d.part.0")["focused"], false);
    presenter
        .intent(
            &mut pm_panel_inspector::Inspector::new(
                kernel.published("synthetic").unwrap(),
                Default::default(),
            ),
            Caller::native(),
            &json!({"type":"collapse","section":"3d.part.1","collapsed":true}),
            0.,
        )
        .unwrap();
    presenter.focus_part(&json!({"layer":"model","slot":1,"seq":1}));
    let p = publish(&mut presenter);
    assert_eq!(section(&p, "3d.part.1")["collapsed"], true);
    // Leaving the model leaves its part.
    kernel
        .call(
            "synthetic",
            Request::new(Caller::native(), pm_kernel::SelectionCall::Clear),
        )
        .wait()
        .unwrap()
        .unwrap();
    let mut model = pm_panel_inspector::Inspector::new(
        kernel.published("synthetic").unwrap(),
        Default::default(),
    );
    presenter.snapshot(&mut model, 0., false).unwrap();
    let p = publish(&mut presenter);
    assert_eq!(section(&p, "3d.part.1")["focused"], false);
}

/// Playing, Properties shows a keyframed value at the playhead: the time the
/// app hands over each display tick (`follow_time`) republishes that panel.
#[test]
fn properties_follow_the_playhead_while_playing() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let project = pm_model::json::parse(r##"{"id":"synthetic","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"a","type":"shape","name":"Headline","from":0,"dur":10,"on":true,"lock":false,"p":{"opacity":{"v":100,"kf":[{"t":0,"v":0},{"t":10,"v":100}]},"position.x":{"v":320,"kf":[]},"position.y":{"v":180,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"##).unwrap();
    let kernel = actor.handle();
    kernel
        .open("synthetic", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    select_layer(&kernel, "a");
    let mut panels = Builtins::new(kernel.clone(), "synthetic", || {}).unwrap();
    panels.update_facts(json!({"time":0,"playing":false,"stage":[640.,360.]}));
    finish(panels.mount("inspector").unwrap());
    let opacity = |panels: &Builtins| {
        let publication = panels.take_publications().pop()?;
        publication
            .surface
            .nodes()
            .values()
            .find(|n| n.props.get("id").and_then(Value::as_str) == Some("value:opacity:0"))
            .map(|n| Value::Object(n.props.clone()))
    };
    let first = opacity(&panels).expect("an opacity field");
    assert!(first.to_string().contains("\"value\":0"), "{first}");
    for time in [2.5, 5.] {
        finish(
            panels
                .follow_time("inspector", time, true)
                .unwrap()
                .unwrap(),
        );
        let shown = opacity(&panels).expect("Properties published while playing");
        let wanted = format!("\"value\":{}", time * 10.);
        assert!(shown.to_string().contains(&wanted), "{time}: {shown}");
    }
    assert!(panels.follow_time("not-open", 1., true).is_none());
    assert!(panels.take_errors().is_empty());
}

/// Properties' Parent and Track Matte rows carry only what the closed
/// pop-up shows (None and the current layer) and the source the host lists
/// the rest from when the menu opens: no list of every layer is built,
/// copied or sent with each publication.
#[test]
fn parent_and_track_matte_rows_carry_only_the_current_layer() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let mut layers = vec![];
    for i in 0..300 {
        let parent = if i == 1 { r#""a0""# } else { "null" };
        layers.push(format!(
            r##"{{"id":"a{i}","type":"solid","name":"Layer {i}","from":0,"dur":10,"on":true,"parent":{parent},
            "d":{{"w":10,"h":10,"color":"#ffffff"}},"p":{{"opacity":{{"v":100,"kf":[]}}}},"fx":[]}}"##
        ));
    }
    let project = format!(
        r##"{{"id":"synthetic","name":"Synthetic","compId":"c","w":64,"h":36,"fps":30,"dur":10,"bg":"#000000",
        "assets":{{}},"comps":{{}},"layers":[{}]}}"##,
        layers.join(",")
    );
    let kernel = actor.handle();
    kernel
        .open(
            "synthetic",
            OpenSource::Untitled(pm_model::json::parse(&project).unwrap()),
        )
        .wait()
        .unwrap()
        .unwrap();
    kernel
        .call(
            "synthetic",
            Request::new(
                Caller::native(),
                SelectionCall::Layers {
                    ids: vec!["a1".into()],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let identity = pm_surface::host_api::Identity {
        realm: "inspector".into(),
        generation: 1,
        project: "synthetic".into(),
    };
    let mut backend = data::Backend::new(kernel.clone(), identity, "inspector", false).unwrap();
    let snapshot = backend.inspector_snapshot().unwrap();
    let row = |id: &str| {
        snapshot["sections"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["rows"].as_array().unwrap())
            .find(|r| r["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} row"))
    };
    let parent = row("layer.parent");
    assert_eq!(parent["control"]["value"], "a0");
    assert_eq!(
        parent["control"]["options"],
        json!([{"v":null,"label":"None"},{"v":"a0","label":"Layer 0"}])
    );
    assert_eq!(
        parent["control"]["rows"],
        json!({"source":"layer-parents","layer":"a1"})
    );
    let matte = row("layer.matteSource");
    assert_eq!(
        matte["control"]["options"],
        json!([{"v":null,"label":"None"}])
    );
    assert_eq!(
        matte["control"]["rows"],
        json!({"source":"layer-mattes","layer":"a1"})
    );
    // The whole projection stays small with 300 layers.
    assert!(
        snapshot.to_string().len() < 20_000,
        "{}",
        snapshot.to_string().len()
    );
}
