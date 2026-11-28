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
    if pm_builtins::builtin_panel(panel).is_some_and(|p| p.window) {
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

#[test]
fn extension_ui_surfaces_mount_collect_menus_and_revert_timeline_chrome() {
    use pm_builtins::{BUILTIN_SLOTS, SLOT_PREFIX};
    use serde_json::json;
    use std::collections::BTreeSet;

    // Check the actual host sources, so adding a public broker slot without
    // routing it to an editor, window or built-in body fails this test.
    let slot_hosts = [
        include_str!("../../shell/src/view.rs"),
        include_str!("../../shell/src/welcome_view.rs"),
        include_str!("builtins.rs"),
        include_str!("project_library.rs"),
        include_str!("panel_library.rs"),
        include_str!("store_window.rs"),
    ];
    for slot in BUILTIN_SLOTS {
        let literal = format!("\"{slot}\"");
        assert!(
            slot_hosts.iter().any(|host| host.contains(&literal)),
            "{slot} has no host"
        );
    }

    let menu_locations = [
        "assets:context",
        crate::builtins::panel_context_location("inspector").unwrap(),
        crate::builtins::panel_context_location("fxbrowser").unwrap(),
        "home:context",
        "toolbar:context",
        "statusBar:context",
    ];
    assert_eq!(
        crate::builtins::panel_context_location("layer-effects"),
        Some("effects:context"),
    );
    assert_eq!(crate::builtins::panel_context_location("settings"), None);

    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    let project = pm_model::json::parse(
        r##"{"id":"integration","name":"Synthetic Film","compId":"c","w":640,"h":360,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[{"id":"a","type":"shape","name":"Headline","from":0,"dur":10,"on":true,"p":{"opacity":{"v":100,"kf":[]}},"d":{"w":100,"h":100},"fx":[]}]}"##,
    ).unwrap();
    kernel
        .open("integration", OpenSource::Untitled(project))
        .wait()
        .unwrap()
        .unwrap();
    let mut runtime = Builtins::new(kernel.clone(), "integration", || {}).unwrap();
    settle(runtime.mount("timeline").unwrap());
    runtime.take_publications();

    let graph = |badge, density| {
        let source = r#"
            import { jsx } from 'powermove/jsx-runtime';
            export function activate({ commands, slots, menus, panelHeader, toolbar, statusBar, timeline, ui }) {
                function reject(api, id, action) {
                    try { action(); } catch { return; }
                    throw Error(api + ' accepted ' + id);
                }
                for (const id of ['settings', 'settings.general', 'panel:settings.top']) {
                    reject('slots.contribute', id, () => slots.contribute(id, () => null));
                    reject('panelHeader.contribute', id, () => panelHeader.contribute(id, { id: 'x', onClick() {} }));
                    reject('panelHeader.modify', id, () => panelHeader.modify(id, { hidden: true }));
                    reject('toolbar.modify', id, () => toolbar.modify(id, { hidden: true }));
                    reject('statusBar.modify', id, () => statusBar.modify(id, { hidden: true }));
                    reject('menus.contribute', id, () => menus.contribute(id, () => []));
                }
                const ids = SLOTS;
                const supported = ui.surfaces().slots;
                for (let start = 0; start < ids.length; start += 11) {
                    commands.register({ id: 'slots-' + start, title: 'Register test slots' }, () => {
                        for (const id of ids.slice(start, start + 11)) {
                            if (!supported.includes(id)) throw Error('Missing slot: ' + id);
                            slots.contribute(id, () => jsx('text', { children: id }));
                        }
                    });
                }
                for (const location of MENUS)
                    menus.contribute(location, (ctx) => [{ label: location + ' ' + ctx.target, run() {} }]);
                timeline.chrome(CHROME);
            }
        "#.replace("SLOTS", &json!(BUILTIN_SLOTS.as_slice()).to_string())
          .replace("MENUS", &json!(menu_locations).to_string())
          .replace("CHROME", &json!({"showTypeBadges": badge, "toolbarDensity": density}).to_string());
        json!({
            "sources": {"powermove:bundle": "export * from './index.tsx';", "index.tsx": source},
            "edges": {
                "powermove:bundle": {"./index.tsx": "index.tsx"},
                "index.tsx": {"powermove/jsx-runtime": "runtime:powermove/jsx-runtime"}
            }
        })
    };
    let publish_timeline = |badges: bool, spacing: &str| {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut slots = BTreeSet::new();
        let mut observed = None;
        loop {
            for (_, ticket) in runtime.publish() {
                settle(ticket.unwrap());
            }
            for publication in runtime.take_publications() {
                if let Some(slot) = publication
                    .panel
                    .strip_prefix(&format!("surface-test{SLOT_PREFIX}"))
                {
                    assert!(publication.surface.nodes().values().any(|n| n.text == slot));
                    slots.insert(slot.to_owned());
                } else if publication.panel == "timeline" {
                    let nodes = publication.surface.nodes();
                    let has_badge = nodes.values().any(|node| {
                        node.props
                            .get("metadata")
                            .is_some_and(|meta| meta["recipe"] == "kind-badge")
                    });
                    let header_spacing = nodes
                        .values()
                        .find(|node| {
                            node.props
                                .get("id")
                                .is_some_and(|id| id == "timeline:header")
                        })
                        .and_then(|header| header.props.get("style"))
                        .map(|style| style["paddingLeft"].clone());
                    observed = Some((has_badge, header_spacing.clone()));
                    if has_badge == badges
                        && header_spacing
                            .as_ref()
                            .is_some_and(|padding| padding["token"] == spacing)
                    {
                        return slots;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "Timeline did not draw badges={badges}, spacing={spacing}. Observed {observed:?}, workspace {}, runtime errors {:?}",
                kernel
                    .published("integration")
                    .map_or_else(String::new, |p| pm_model::json::to_canonical_string(
                        &p.workspace
                    )),
                runtime.take_errors(),
            );
            thread::sleep(Duration::from_millis(5));
        }
    };

    settle(
        runtime
            .mount_extension("surface-test", &graph(false, "compact"), None)
            .unwrap(),
    );
    // Separate evaluations keep registration of every slot within V8's
    // production per-turn engine operation limit.
    for start in (0..BUILTIN_SLOTS.len()).step_by(11) {
        settle(
            runtime
                .run_command("surface-test", &format!("slots-{start}"), &[])
                .unwrap(),
        );
    }
    let published = publish_timeline(false, "space.xs");
    assert_eq!(
        published,
        BUILTIN_SLOTS.into_iter().map(str::to_owned).collect()
    );
    for location in menu_locations {
        let entries =
            crate::extensions::v4_menu_items(&runtime, location, &json!({"target":"Synthetic"}));
        assert_eq!(entries.len(), 1, "{location}");
        assert_eq!(entries[0].label, format!("{location} Synthetic"));
        assert!(entries[0].enabled && !entries[0].command.is_empty());
    }
    for id in ["settings", "settings.general", "panel:settings.top"] {
        assert!(pm_builtins::is_protected_surface(id));
        assert!(!pm_builtins::is_valid_slot(id));
        assert!(
            runtime
                .replace(id, "surface-test", &graph(false, "compact"))
                .is_err()
        );
        assert!(runtime.gather_menus(id, &json!({})).is_empty());
        assert!(
            runtime
                .gather_menus("panel:context", &json!({"panelId":id}))
                .is_empty()
        );
    }
    runtime.unmount_extension("surface-test");
    assert_eq!(
        runtime
            .take_cleared_slots()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        BUILTIN_SLOTS
            .into_iter()
            .map(|slot| format!("surface-test{SLOT_PREFIX}{slot}"))
            .collect()
    );
    assert!(crate::extensions::v4_menu_items(&runtime, "assets:context", &json!({})).is_empty());
    publish_timeline(true, "metrics.gap");

    settle(
        runtime
            .mount_extension("surface-test", &graph(true, "comfortable"), None)
            .unwrap(),
    );
    publish_timeline(true, "space.md");
    runtime.unmount_extension("surface-test");
    publish_timeline(true, "metrics.gap");
    runtime.close();
    actor.shutdown();
}
