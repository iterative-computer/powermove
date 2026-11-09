//! Application host integration tests. These instantiate the production window
//! runtime with a real kernel actor and inspect accepted UI-kit/AX publications.
use pm_builtins::Builtins;
use pm_kernel::{
    Call, Caller, Command, KernelActor, KernelConfig, LayerCommand, OpenSource, Request,
    SelectionCall, WriteBody,
};
use pm_runtime_v8::{Evaluation, Ticket};
use std::{
    thread,
    time::{Duration, Instant},
};
fn settle(ticket: Ticket<Evaluation>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(result) = ticket.try_take() {
            result.expect("production built-in V8 entry");
            return;
        }
        assert!(Instant::now() < deadline, "panel did not settle");
        thread::sleep(Duration::from_millis(1));
    }
}
fn check(panel: &str, title: &str) {
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let project=pm_model::json::parse(r##"{"id":"integration","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"a","type":"shape","name":"Headline","from":0,"dur":10,"on":true,"p":{"opacity":{"v":100,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"##).unwrap();
    kernel
        .open("integration", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    kernel
        .call(
            "integration",
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
    let mut runtime = Builtins::new(kernel.clone(), "integration", || {}).unwrap();
    settle(runtime.mount(panel).unwrap());
    if matches!(panel, "settings" | "export") {
        settle(runtime.show(panel).unwrap());
    }
    let first = runtime
        .take_publications()
        .pop()
        .expect("accepted initial surface");
    for scheme in [pm_design::Scheme::Light, pm_design::Scheme::Dark] {
        let kit = pm_surface::kit::Panel::prepare(
            first.surface.clone(),
            &pm_design::Tokens::new(scheme, Default::default()),
            &Default::default(),
            None,
            false,
        );
        assert!(kit.diagnostics.is_empty(), "{panel}: {:?}", kit.diagnostics);
        assert!(
            kit.accessibility
                .nodes
                .iter()
                .any(|(_, n)| n.label().is_some_and(|label| label.contains(title))),
            "{panel} missing named AX group {title}"
        );
    }
    assert!(
        runtime.status()[0]["subscriptions"].as_u64().unwrap() > 0,
        "{panel} did not bind through HostDataService"
    );
    kernel
        .call(
            "integration",
            Request::new(
                Caller::native(),
                Call::Transact {
                    label: Some("Rename test layer".into()),
                    body: WriteBody::Commands(vec![Command::Layer(LayerCommand::Set {
                        layer_id: "a".into(),
                        patch: pm_kernel::commands::layers::LayerPatch {
                            name: Some("Updated Headline".into()),
                            ..Default::default()
                        },
                    })]),
                    expected_revision: None,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let revision = kernel.published("integration").unwrap().revision();
    for (_, ticket) in runtime.publish() {
        settle(ticket.unwrap());
    }
    assert_eq!(
        runtime.status()[0]["kernelRevision"].as_u64(),
        Some(revision),
        "{panel} did not consume the new kernel publication"
    );
    assert!(runtime.take_errors().is_empty(), "{panel} worker errors");
    runtime.close();
}
macro_rules! panel_test {
    ($name:ident,$panel:literal,$title:literal) => {
        #[test]
        fn $name() {
            check($panel, $title);
        }
    };
}
panel_test!(
    timeline_mounts_binds_updates_and_exposes_ax,
    "timeline",
    "Timeline"
);
panel_test!(
    properties_mounts_binds_updates_and_exposes_ax,
    "inspector",
    "Properties"
);
panel_test!(
    effect_controls_mounts_binds_updates_and_exposes_ax,
    "layer-effects",
    "Effect Controls"
);
panel_test!(
    viewer_mounts_binds_updates_and_exposes_ax,
    "viewer",
    "Viewer"
);
panel_test!(
    project_mounts_binds_updates_and_exposes_ax,
    "assets",
    "Project"
);
panel_test!(
    effects_mounts_binds_updates_and_exposes_ax,
    "fxbrowser",
    "Effects"
);
panel_test!(agent_mounts_binds_updates_and_exposes_ax, "agent", "Agent");
panel_test!(
    settings_mounts_binds_updates_and_exposes_ax,
    "settings",
    "Settings"
);
panel_test!(
    export_mounts_binds_updates_and_exposes_ax,
    "export",
    "Export"
);

#[::core::prelude::v1::test]
fn timeline_chrome_follows_the_workspace_and_the_theme_surface() {
    use pm_design::{Accessibility, Scheme, Tokens};
    let base = Tokens::new(Scheme::Dark, Accessibility::default());
    let workspace = pm_model::json::parse(
        r#"{"chrome":{"timeline":{"rowHeight":30,"gutterWidth":900,"rulerHeight":28,"keyframeSize":7.5,"clipRadius":9},"timelineSurfaceOrder":"normal"}}"#,
    )
    .unwrap();
    let t = crate::builtins::timeline_chrome(base.clone(), &workspace, &serde_json::Value::Null);
    assert_eq!(t.metrics.track_row, 30.);
    assert_eq!(t.metrics.layer_column, 640., "v1's range");
    assert_eq!(
        (t.metrics.ruler, t.metrics.keyframe),
        (28., 7.5),
        "keys keep fractions"
    );
    assert_eq!(
        t.metrics.clip_radius, 5.,
        "main's clip bars cap the corner at 5"
    );
    // v1's written default gives way to the native column; another width is kept.
    let gutter = |w: f64| {
        let ws = pm_model::json::parse(&format!(
            r#"{{"chrome":{{"timeline":{{"gutterWidth":{w}}}}}}}"#
        ))
        .unwrap();
        crate::builtins::timeline_chrome(base.clone(), &ws, &serde_json::Value::Null)
            .metrics
            .layer_column
    };
    assert_eq!(gutter(224.), base.metrics.layer_column);
    assert_eq!(gutter(300.), 300.);
    let clips = pm_model::json::parse(r#"{"chrome":{"timeline":{"clipRadius":2.5}}}"#).unwrap();
    let t2 = crate::builtins::timeline_chrome(base.clone(), &clips, &serde_json::Value::Null);
    assert_eq!(t2.metrics.clip_radius, 2.5);
    let surface = serde_json::json!({"track":0x202020,"gutter":0x101010,"accent":0x123456});
    let t = crate::builtins::timeline_chrome(base.clone(), &workspace, &surface);
    assert_eq!(
        (t.colors.panel, t.colors.panel_secondary, t.colors.accent),
        (0x202020, 0x101010, 0x123456)
    );
    let reversed =
        pm_model::json::parse(r#"{"chrome":{"timelineSurfaceOrder":"reversed"}}"#).unwrap();
    let t = crate::builtins::timeline_chrome(base.clone(), &reversed, &surface);
    assert_eq!(
        (t.colors.panel, t.colors.panel_secondary),
        (0x101010, 0x202020)
    );
}
