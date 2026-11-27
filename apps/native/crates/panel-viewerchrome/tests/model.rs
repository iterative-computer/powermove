use pm_kernel::{
    Call, Caller, Command, CompCommand, HistoryCall, KernelActor, KernelConfig, OpenSource, Origin,
    Reply, Request, WriteBody,
    state::{StateOp, StateTransaction},
};
use pm_model::{Value, json::parse};
use pm_panel_viewerchrome::*;
const FIXTURE: &str = r##"{"id":"p","name":"Synthetic","compId":"c","w":1920,"h":1080,"fps":24,"dur":10,"work":[1,9],"bg":"#000000","layers":[{"id":"a","name":"A","type":"shape","from":0,"dur":10,"on":true,"p":{"opacity":{"v":100,"kf":[]}}},{"id":"b","name":"B","type":"shape","from":1,"dur":5,"lock":true,"p":{"opacity":{"v":100,"kf":[]}}}],"assets":{"video":{"kind":"video","name":"Synthetic"}},"comps":{}}"##;
fn fixture() -> (KernelActor, Controller) {
    let actor = KernelActor::spawn(KernelConfig::default());
    actor
        .handle()
        .open("p", OpenSource::Untitled(parse(FIXTURE).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let controller = Controller::new(actor.handle(), "p");
    (actor, controller)
}
fn apply(controller: &Controller, action: Action) {
    let model = controller.read().unwrap();
    let plan = model.plan(action, Caller::native()).unwrap();
    finish(
        controller
            .submit(plan, &mut InspectHost::default())
            .unwrap(),
    );
}
fn finish(submission: Submission) {
    match submission {
        Submission::State(pending) => {
            pending.wait().unwrap().unwrap();
        }
        Submission::Document(pending) => {
            let ack = pending.wait().unwrap().unwrap();
            assert!(!matches!(ack.reply, Reply::Throw(_)), "{:?}", ack.reply);
        }
        Submission::Enqueued => panic!("expected kernel submission"),
    }
}
fn ui_edit(actor: &KernelActor, action: impl FnOnce(&mut pm_kernel::state::UiState)) {
    let state = actor.handle().state();
    let mut ui = actor.handle().published("p").unwrap().ui_state.clone();
    action(&mut ui);
    actor
        .handle()
        .state_transaction(StateTransaction {
            expected_revision: Some(state.revision),
            ops: vec![StateOp::Ui {
                project: "p".into(),
                next: ui,
            }],
        })
        .wait()
        .unwrap()
        .unwrap();
}
#[test]
fn model_reads_kernel_comp_selection_and_defaults() {
    let (_actor, controller) = fixture();
    let model = controller.read().unwrap();
    assert_eq!(model.composition.as_deref(), Some("c"));
    assert_eq!(model.size.width, 1920.0);
    assert_eq!(model.transport.work, (1.0, 9.0));
    assert_eq!(model.transport.fps, 24.0);
    assert_eq!(model.chrome.quality, Quality::Auto);
    assert!(model.chrome.show_controls);
    assert!(model.chrome.overlays.guides.is_empty());
}
#[test]
fn view_changes_leave_document_revision_dirty_and_history_untouched() {
    let (actor, controller) = fixture();
    let before = controller.published().unwrap();
    for action in [
        Action::SetZoom(2.0),
        Action::ToggleControls,
        Action::Quality(Quality::Half),
        Action::ToggleGuides,
        Action::ToggleSafeAreas,
        Action::Fit,
    ] {
        apply(&controller, action);
    }
    let after = controller.published().unwrap();
    assert_eq!(before.revision(), after.revision());
    assert_eq!(before.dirty, after.dirty);
    assert_eq!(before.history.entries, after.history.entries);
    let model = controller.read().unwrap();
    assert!(model.chrome.navigation.fit);
    assert!(!model.chrome.show_controls);
    assert!(model.chrome.overlays.guides_visible);
    assert!(model.chrome.overlays.safe_areas_visible);
    assert_eq!(model.chrome.quality, Quality::Half);
    assert_eq!(model.chrome.effective_quality, 0.5);
    actor.shutdown();
}
#[test]
fn opaque_compat_fields_survive_all_view_and_quality_changes() {
    let (actor, controller) = fixture();
    ui_edit(&actor, |ui| {
        ui.viewer
            .as_object_mut()
            .unwrap()
            .insert("extensionOwned", parse(r#"{"future":[1,2,3]}"#).unwrap());
        ui.transport.as_object_mut().unwrap().insert(
            "perf",
            parse(r#"{"auto":false,"decoder":"opaque"}"#).unwrap(),
        );
    });
    apply(&controller, Action::Quality(Quality::Quarter));
    apply(&controller, Action::SetZoom(1.5));
    apply(&controller, Action::Quality(Quality::Auto));
    let p = controller.published().unwrap();
    assert_eq!(
        p.ui_state.viewer.get("extensionOwned"),
        Some(&parse(r#"{"future":[1,2,3]}"#).unwrap())
    );
    assert_eq!(
        p.ui_state
            .transport
            .get("perf")
            .unwrap()
            .get("decoder")
            .and_then(Value::as_str),
        Some("opaque")
    );
    assert_eq!(controller.read().unwrap().chrome.effective_quality, 1.0);
}
#[test]
fn work_area_is_one_kernel_transaction_with_undo_and_redo() {
    let (actor, controller) = fixture();
    apply(&controller, Action::Transport(TransportAction::Seek(4.0)));
    let before = controller.published().unwrap();
    apply(&controller, Action::WorkIn);
    assert_eq!(controller.read().unwrap().transport.work, (4.0, 9.0));
    assert_eq!(
        controller.published().unwrap().history.entries,
        before.history.entries + 1
    );
    for (call, work) in [
        (HistoryCall::Undo, (1.0, 9.0)),
        (HistoryCall::Redo, (4.0, 9.0)),
    ] {
        actor
            .handle()
            .call("p", Request::new(Caller::native(), call))
            .wait()
            .unwrap()
            .unwrap();
        assert_eq!(controller.read().unwrap().transport.work, work);
    }
    apply(&controller, Action::ResetWork);
    assert_eq!(controller.read().unwrap().transport.work, (0.0, 10.0));
}
#[test]
fn work_bounds_keep_a_frame_and_work_out_undo_restores() {
    let (actor, controller) = fixture();
    apply(&controller, Action::Transport(TransportAction::Start));
    apply(&controller, Action::WorkOut);
    let model = controller.read().unwrap();
    assert!((model.transport.work.1 - model.transport.work.0 - 1.0 / 24.0).abs() < 1e-8);
    actor
        .handle()
        .call("p", Request::new(Caller::native(), HistoryCall::Undo))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(controller.read().unwrap().transport.work, (1.0, 9.0));
}
#[test]
fn stale_view_plans_cannot_overwrite_foreign_facts() {
    let (actor, controller) = fixture();
    let model = controller.read().unwrap();
    let plan = model.plan(Action::Fit, Caller::native()).unwrap();
    ui_edit(&actor, |ui| {
        ui.viewer
            .as_object_mut()
            .unwrap()
            .insert("activePath", Value::from("foreign"));
    });
    assert!(matches!(
        controller.submit(plan, &mut InspectHost::default()),
        Err(Error::Stale)
    ));
    assert_eq!(
        controller
            .published()
            .unwrap()
            .ui_state
            .viewer
            .get("activePath")
            .and_then(Value::as_str),
        Some("foreign")
    );
}
#[test]
fn actor_rejects_queued_state_plan_after_a_foreign_state_transaction() {
    let (actor, controller) = fixture();
    let model = controller.read().unwrap();
    let plan = model.plan(Action::SetZoom(3.0), Caller::native()).unwrap();
    let Effect::State(tx) = plan.effect else {
        panic!()
    };
    ui_edit(&actor, |ui| {
        ui.tool
            .as_object_mut()
            .unwrap()
            .insert("tool", Value::from("pen"));
    });
    assert!(
        actor
            .handle()
            .state_transaction(tx)
            .wait()
            .unwrap()
            .is_err()
    );
    assert!(controller.read().unwrap().chrome.navigation.fit);
}
#[test]
fn stale_document_plan_is_rejected_by_actor_transaction() {
    let (actor, controller) = fixture();
    let plan = controller
        .read()
        .unwrap()
        .plan(Action::WorkIn, Caller::native())
        .unwrap();
    actor
        .handle()
        .call(
            "p",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: None,
                    body: WriteBody::Commands(vec![Command::Comp(CompCommand::ResetWorkArea {
                        comp: Some("c".into()),
                    })]),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let Effect::Document(request) = plan.effect else {
        panic!()
    };
    let ack = actor.handle().call("p", request).wait().unwrap().unwrap();
    assert!(matches!(ack.reply, Reply::Throw(_)));
    assert_eq!(controller.read().unwrap().transport.work, (0.0, 10.0));
}
#[test]
fn reopened_document_with_same_id_invalidates_old_plans() {
    let (actor, controller) = fixture();
    let plan = controller
        .read()
        .unwrap()
        .plan(Action::Quality(Quality::Full), Caller::native())
        .unwrap();
    actor.handle().close("p").wait().unwrap();
    actor
        .handle()
        .open("p", OpenSource::Untitled(parse(FIXTURE).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(
        controller.submit(plan, &mut InspectHost::default()),
        Err(Error::Stale)
    ));
}
#[test]
fn selection_ids_replace_add_toggle_without_index_assumptions() {
    let (_actor, controller) = fixture();
    for (ids, mode, expected) in [
        (vec!["a"], SelectionMode::Replace, vec!["a"]),
        (vec!["b"], SelectionMode::Add, vec!["a", "b"]),
        (vec!["a"], SelectionMode::Toggle, vec!["b"]),
    ] {
        apply(
            &controller,
            Action::Select {
                ids: ids.into_iter().map(str::to_owned).collect(),
                mode,
            },
        );
        assert_eq!(
            controller.read().unwrap().selection.layers(),
            expected.as_slice()
        );
    }
    assert!(
        controller
            .read()
            .unwrap()
            .plan(
                Action::Select {
                    ids: vec!["missing".into()],
                    mode: SelectionMode::Replace
                },
                Caller::native()
            )
            .is_err()
    );
}
#[test]
fn headless_ids_match_commands_and_bad_ids_never_mutate() {
    let (_actor, controller) = fixture();
    let mut host = InspectHost::default();
    let model = controller.read().unwrap();
    let a = model
        .drive(
            "viewer.zoom",
            Value::Number(2.0),
            None,
            Caller::realm(7, 1, Origin::Mcp),
        )
        .unwrap();
    finish(controller.submit(a, &mut host).unwrap());
    assert_eq!(controller.read().unwrap().chrome.navigation.zoom, 2.0);
    let model = controller.read().unwrap();
    for (id, arg) in [
        ("viewer.zoom", Value::Number(f64::NAN)),
        ("viewer.transport.step", Value::Number(1.5)),
        ("viewer.fit", Value::Bool(true)),
        ("viewer.quality", Value::from("fake")),
        ("viewer.unknown", Value::Null),
    ] {
        assert!(model.drive(id, arg, None, Caller::native()).is_err());
    }
    assert!(host.requests.is_empty());
}
#[test]
fn context_actions_include_locked_ids_and_preserve_selection() {
    let (_actor, controller) = fixture();
    let model = controller.read().unwrap();
    let mut host = InspectHost::default();
    for layer in [Some("b".into()), None] {
        let plan = model.context_menu(layer.clone()).unwrap();
        assert!(matches!(
            controller.submit(plan, &mut host).unwrap(),
            Submission::Enqueued
        ));
        assert_eq!(
            host.requests.last().unwrap().1,
            HostAction::ContextMenu { layer, time: 0.0 }
        );
    }
    assert!(controller.read().unwrap().selection.is_empty());
    assert!(model.context_menu(Some("missing".into())).is_err());
}
#[test]
fn media_intent_is_enqueued_and_never_claims_playback_before_host_acceptance() {
    let (_actor, controller) = fixture();
    let model = controller.read().unwrap();
    let mut host = InspectHost::default();
    let plan = model
        .drive("viewer.transport.play", Value::Null, None, Caller::native())
        .unwrap();
    assert!(matches!(
        controller.submit(plan, &mut host).unwrap(),
        Submission::Enqueued
    ));
    assert!(!controller.read().unwrap().transport.playing);
    assert_eq!(
        host.requests[0].1,
        HostAction::Transport(TransportAction::Toggle)
    );
    for (id, arg) in [
        ("viewer.source.show", Value::from("video")),
        ("viewer.scene.transform", Value::from("rotate")),
        ("viewer.source.seek", Value::Number(0.5)),
    ] {
        controller
            .submit(
                model.drive(id, arg, None, Caller::native()).unwrap(),
                &mut host,
            )
            .unwrap();
    }
    assert_eq!(host.requests.len(), 4);
    assert!(
        model
            .drive(
                "viewer.source.show",
                Value::from("unknown"),
                None,
                Caller::native()
            )
            .is_err()
    );
}
#[test]
fn query_mirror_is_bound_to_document_lifetime() {
    let (actor, controller) = fixture();
    let old = controller.published().unwrap();
    let mut queries = ViewerQueries::new(&old);
    actor.handle().close("p").wait().unwrap();
    actor
        .handle()
        .open("p", OpenSource::Untitled(parse(FIXTURE).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    let model = controller.read().unwrap();
    assert!(matches!(
        model.pick(&mut queries, Point::default(), false),
        Err(Error::Stale)
    ));
    queries.sync(&controller.published().unwrap());
    // Missing font/model inputs remain typed query errors; not coerced into empty.
    assert!(!matches!(
        model.pick(&mut queries, Point::default(), false),
        Err(Error::Stale)
    ));
}
#[test]
fn sixty_thousand_selected_ids_have_bounded_inspection_and_linear_selection_planning() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let mut project = parse(FIXTURE).unwrap();
    let template = project.get("layers").unwrap().as_array().unwrap()[0].clone();
    let mut layers = Vec::with_capacity(60_000);
    let mut ids = Vec::with_capacity(60_000);
    for n in 0..60_000 {
        let id = format!("layer-{n}");
        let mut layer = template.clone();
        layer
            .as_object_mut()
            .unwrap()
            .insert("id", Value::from(id.as_str()));
        layers.push(layer);
        ids.push(id);
    }
    project
        .as_object_mut()
        .unwrap()
        .insert("layers", Value::Array(layers));
    actor
        .handle()
        .open("p", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let controller = Controller::new(actor.handle(), "p");
    apply(
        &controller,
        Action::Select {
            ids,
            mode: SelectionMode::Replace,
        },
    );
    let model = controller.read().unwrap();
    assert_eq!(model.selection.layers().len(), 60_000);
    assert_eq!(model.selection_page(59_990, 100).len(), 10);
    assert_eq!(model.selection_page(0, usize::MAX).len(), 1024);
    assert_eq!(model.selection_page(59_999, 100)[0], "layer-59999");
    assert!(model.selection_page(usize::MAX, 100).is_empty());
    assert!(model.chrome.overlays.guides.is_empty());
}
#[test]
fn group_selection_deep_pick_and_double_click_descend_one_level() {
    let actor = KernelActor::spawn(KernelConfig::default());
    let mut project = parse(FIXTURE).unwrap();
    project.as_object_mut().unwrap().insert("layers",parse(r#"[{"id":"outer","type":"group","from":0,"dur":10},{"id":"inner","type":"group","group":"outer","from":0,"dur":10},{"id":"leaf","type":"shape","group":"inner","from":0,"dur":10},{"id":"sibling","type":"shape","group":"inner","from":0,"dur":10}]"#).unwrap());
    actor
        .handle()
        .open("p", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let controller = Controller::new(actor.handle(), "p");
    let m = controller.read().unwrap();
    assert_eq!(
        m.pick_target(Some("leaf"), false, false)
            .unwrap()
            .as_deref(),
        Some("outer")
    );
    assert_eq!(
        m.pick_target(Some("leaf"), true, false).unwrap().as_deref(),
        Some("leaf")
    );
    apply(
        &controller,
        Action::Select {
            ids: vec!["outer".into()],
            mode: SelectionMode::Replace,
        },
    );
    assert_eq!(
        controller
            .read()
            .unwrap()
            .pick_target(Some("leaf"), false, true)
            .unwrap()
            .as_deref(),
        Some("inner")
    );
    apply(
        &controller,
        Action::Select {
            ids: vec!["leaf".into()],
            mode: SelectionMode::Replace,
        },
    );
    assert_eq!(
        controller
            .read()
            .unwrap()
            .pick_target(Some("sibling"), false, false)
            .unwrap()
            .as_deref(),
        Some("sibling")
    );
    assert_eq!(
        controller
            .read()
            .unwrap()
            .pick_target(Some("leaf"), false, true)
            .unwrap(),
        None
    );
}
#[test]
fn every_inspected_element_id_is_drivable_and_identity_is_lossless() {
    let (_actor, controller) = fixture();
    let model = controller.read().unwrap();
    let inspection = model.inspect();
    assert_eq!(
        inspection
            .get("elements")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        ELEMENT_IDS.len()
    );
    assert!(inspection.get("document").unwrap().as_str().is_some());
    let viewport = model
        .viewport(
            Size {
                width: 800.0,
                height: 600.0,
            },
            8.0,
        )
        .unwrap();
    for &id in ELEMENT_IDS {
        let arg = match id {
            "viewer.zoom" => Value::Number(0.5),
            "viewer.quality" => Value::from("auto"),
            "viewer.transport.seek" => Value::Number(1.0),
            "viewer.transport.step" => Value::Number(1.0),
            "viewer.source.show" | "viewer.source.toggle" => Value::from("video"),
            "viewer.source.seek" => Value::Number(0.5),
            "viewer.selection" | "viewer.selection.add" | "viewer.selection.toggle" => {
                Value::Array(vec![Value::from("a")])
            }
            "viewer.scene.transform" => Value::from("rotate"),
            "viewer.scene.axes" => Value::from("local"),
            "viewer.scene.view" => Value::from("editor"),
            "viewer.scene.navigation" => Value::from("frame-selection"),
            _ => Value::Null,
        };
        assert!(
            model
                .drive(id, arg, Some(viewport), Caller::native())
                .is_ok(),
            "{id}"
        );
    }
}
#[test]
fn selection_only_changes_invalidate_old_toggle_plans_before_enqueue() {
    let (_actor, controller) = fixture();
    let plan = controller
        .read()
        .unwrap()
        .plan(
            Action::Select {
                ids: vec!["a".into()],
                mode: SelectionMode::Toggle,
            },
            Caller::native(),
        )
        .unwrap();
    apply(
        &controller,
        Action::Select {
            ids: vec!["b".into()],
            mode: SelectionMode::Replace,
        },
    );
    assert!(matches!(
        controller.submit(plan, &mut InspectHost::default()),
        Err(Error::Stale)
    ));
    assert_eq!(controller.read().unwrap().selection.layers(), ["b"]);
}
#[test]
fn nonfinite_seek_during_playback_is_rejected_before_host_enqueue() {
    let (actor, controller) = fixture();
    ui_edit(&actor, |ui| {
        ui.transport
            .as_object_mut()
            .unwrap()
            .insert("playing", Value::Bool(true));
    });
    assert!(
        controller
            .read()
            .unwrap()
            .plan(
                Action::Transport(TransportAction::Seek(f64::NAN)),
                Caller::native()
            )
            .is_err()
    );
}
