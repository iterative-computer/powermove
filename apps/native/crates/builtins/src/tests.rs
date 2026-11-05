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
