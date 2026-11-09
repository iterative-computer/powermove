//! Real bundled v4 entry + V8 worker + kernel actor, using synthetic documents.
use super::*;

#[test]
fn native_timeline_gestures_run_in_bundled_worker_and_dispose_cancels_preview() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("timeline").unwrap());
    finish(
        panels
            .timeline_action(json!({"type":"beginDrag","kind":"layers","ids":["a"]}))
            .unwrap(),
    );
    finish(
        panels
            .timeline_action(json!({"type":"drag","timeDelta":2.}))
            .unwrap(),
    );
    let moved = actor.handle().published("synthetic").unwrap();
    assert_eq!(
        pm_panel_timeline::TimelineIndex::from_snapshot(&moved.snapshot)
            .layer("a")
            .unwrap()
            .from,
        2.
    );
    finish(panels.timeline_action(json!({"type":"dispose"})).unwrap());
    let restored = actor.handle().published("synthetic").unwrap();
    assert_eq!(
        pm_panel_timeline::TimelineIndex::from_snapshot(&restored.snapshot)
            .layer("a")
            .unwrap()
            .from,
        0.
    );
}

#[test]
fn timeline_action_ack_waits_for_native_effect_reply_and_propagates_failure() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("timeline").unwrap());
    let ticket = panels.timeline_action(json!({"type":"play"})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let request = loop {
        assert!(
            ticket.try_take().is_none(),
            "effect acknowledged before native reply"
        );
        if let Some(request) = panels
            .take_platform_requests()
            .into_iter()
            .find(|r| r.operation == "timeline.effect")
        {
            break request;
        }
        assert!(Instant::now() < deadline, "no platform effect");
        thread::sleep(Duration::from_millis(1));
    };
    assert_ne!(request.id, 0);
    assert_eq!(request.input, json!({"kind":"command","name":"play"}));
    panels.platform_reply(request.id, Err("synthetic transport refused".into()));
    let result = loop {
        if let Some(result) = ticket.try_take() {
            break result;
        }
        assert!(Instant::now() < deadline, "no effect rejection");
        thread::sleep(Duration::from_millis(1));
    };
    assert!(format!("{result:?}").contains("synthetic transport refused"));
}

#[test]
fn joined_position_projection_and_selection_keep_all_subframe_key_identities() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("timeline").unwrap());
    finish(
        panels
            .timeline_action(json!({"type":"separateDimensions","layer":"a","separate":false}))
            .unwrap(),
    );
    finish(
        panels
            .timeline_action(json!({"type":"keyToggle","layer":"a","path":"position"}))
            .unwrap(),
    );
    for (_, ticket) in panels.publish() {
        finish(ticket.unwrap());
    }
    let publication = panels
        .take_publications()
        .into_iter()
        .find(|p| p.panel == "timeline")
        .unwrap();
    let index = pm_panel_timeline::TimelineIndex::from_snapshot(
        &actor.handle().published("synthetic").unwrap().snapshot,
    );
    let keys: Vec<_> = ["position.x", "position.y"]
        .iter()
        .flat_map(|path| index.channel("a", path).unwrap().keys.iter())
        .collect();
    assert_eq!(keys.len(), 2, "coincident axes keep distinct stored ids");
    assert_ne!(keys[0].id, keys[1].id);
    finish(
        panels
            .timeline_key("a", "position", &keys[0].id, false)
            .unwrap(),
    );
    let published = actor.handle().published("synthetic").unwrap();
    assert_eq!(published.selection.keys().len(), 2);
    assert!(
        publication
            .surface
            .nodes()
            .values()
            .any(|n| n.text == "320, 180")
    );
}

#[test]
fn timeline_rejects_unimplemented_host_effect_instead_of_acknowledging_success() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("timeline").unwrap());
    let ticket = panels.timeline_action(json!({"type":"import"})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let result = loop {
        if let Some(result) = ticket.try_take() {
            break result;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    };
    assert!(format!("{result:?}").contains("Timeline host effect unavailable"));
    assert!(
        panels
            .take_platform_requests()
            .iter()
            .all(|r| r.operation != "timeline.effect")
    );
}

fn library_source() -> Value {
    let point = |t: f64, v: f64| json!({"t":t,"v":v,"inInterp":"bezier","outInterp":"bezier","inEase":{"speed":0,"influence":33},"outEase":{"speed":0,"influence":33}});
    json!({"version":1,"active":"group","groups":[{"id":"group","name":"Authored Group","presets":[{"id":"preset","name":"Authored Curve","color":"#123456","curve":{"points":[point(0.,0.),point(1.,1.)]},"future":{"retain":true}}]}],"futureLibrary":{"retain":[1,2,3]}})
}
fn library_action(panels: &Builtins, actor: &KernelActor, mut input: Value) -> Ticket<Evaluation> {
    input["type"] = json!("easingLibrary");
    input["stateRevision"] = json!(actor.handle().state().revision.to_string());
    panels.timeline_action(input).unwrap()
}
#[test]
fn easing_service_edits_publish_with_exact_before_images_in_kernel_settings() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("timeline").unwrap());
    let source = library_source();
    finish(library_action(
        &panels,
        &actor,
        json!({"action":"import","source":source}),
    ));
    // Compare exact values at the kernel boundary; JavaScript has one Number type.
    let source = data::wire(&actor.handle().state().settings[crate::timeline_easing::STORAGE])
        [pm_services::surfaces::easing::STORAGE_KEY]
        .clone();
    let revision = actor.handle().published("synthetic").unwrap().revision();
    finish(library_action(
        &panels,
        &actor,
        json!({"action":"rename","id":"preset","name":"Renamed"}),
    ));
    let state = actor.handle().state();
    let current = data::wire(&state.settings[crate::timeline_easing::STORAGE])
        [pm_services::surfaces::easing::STORAGE_KEY]
        .clone();
    let archives = data::wire(&state.settings[crate::timeline_easing::STORAGE])
        [crate::timeline_easing::ARCHIVES]
        .clone();
    assert_eq!(archives, json!([source]));
    assert_eq!(current["futureLibrary"], source["futureLibrary"]);
    assert_eq!(
        current["groups"][0]["presets"][0]["future"],
        source["groups"][0]["presets"][0]["future"]
    );
    assert_eq!(current["groups"][0]["presets"][0]["name"], "Renamed");
    finish(library_action(
        &panels,
        &actor,
        json!({"action":"archive","id":"preset"}),
    ));
    let state = actor.handle().state();
    assert_eq!(
        data::wire(&state.settings[crate::timeline_easing::STORAGE])
            [crate::timeline_easing::ARCHIVES]
            .clone(),
        json!([source, current])
    );
    assert_eq!(
        actor.handle().published("synthetic").unwrap().revision(),
        revision,
        "library is not document history"
    );
    let inspected = finish(panels.timeline_easing_library().unwrap());
    let inspected: Value = serde_json::from_str(&inspected.value).unwrap();
    assert_eq!(
        inspected["stateRevision"],
        actor.handle().state().revision.to_string()
    );
}

#[test]
fn stale_and_malformed_easing_libraries_refuse_without_replacing_source_or_archive() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("timeline").unwrap());
    finish(library_action(
        &panels,
        &actor,
        json!({"action":"import","source":library_source()}),
    ));
    let state = actor.handle().state();
    let before = data::wire(&state.settings[crate::timeline_easing::STORAGE])
        [pm_services::surfaces::easing::STORAGE_KEY]
        .clone();
    let archives = data::wire(&state.settings[crate::timeline_easing::STORAGE])
        [crate::timeline_easing::ARCHIVES]
        .clone();
    for input in [
        json!({"type":"easingLibrary","action":"rename","id":"preset","name":"stale","stateRevision":"0"}),
        json!({"type":"easingLibrary","action":"import","source":{"version":99},"ids":[],"stateRevision":state.revision.to_string()}),
    ] {
        let ticket = panels.timeline_action(input).unwrap();
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
    let state = actor.handle().state();
    assert_eq!(
        data::wire(&state.settings[crate::timeline_easing::STORAGE])
            [pm_services::surfaces::easing::STORAGE_KEY]
            .clone(),
        before
    );
    assert_eq!(
        data::wire(&state.settings[crate::timeline_easing::STORAGE])
            [crate::timeline_easing::ARCHIVES]
            .clone(),
        archives
    );
}

#[test]
fn easing_library_actions_share_service_validation_and_never_drop_prior_versions() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("timeline").unwrap());
    finish(library_action(
        &panels,
        &actor,
        json!({"action":"import","source":library_source()}),
    ));
    let curve = library_source()["groups"][0]["presets"][0]["curve"].clone();
    let mut versions = Vec::new();
    for input in [
        json!({"action":"addGroup","id":"destination","name":"Destination"}),
        json!({"action":"save","group":"destination","id":"saved","name":"Saved","curve":curve}),
        json!({"action":"duplicate","id":"preset","newId":"copy","name":"Copy"}),
        json!({"action":"color","id":"copy","color":"#aabbcc"}),
        json!({"action":"move","id":"copy","group":"destination","before":"saved"}),
        json!({"action":"import","source":library_source(),"ids":["import-group","import-preset"]}),
    ] {
        let state = actor.handle().state();
        versions.push(
            data::wire(&state.settings[crate::timeline_easing::STORAGE])
                [pm_services::surfaces::easing::STORAGE_KEY]
                .clone(),
        );
        finish(library_action(&panels, &actor, input));
        let state = actor.handle().state();
        let stored = data::wire(&state.settings[crate::timeline_easing::STORAGE]);
        assert_eq!(stored[crate::timeline_easing::ARCHIVES], json!(versions));
        pm_services::surfaces::easing::Library::read(
            stored[pm_services::surfaces::easing::STORAGE_KEY].clone(),
        )
        .unwrap();
    }
    let state = actor.handle().state();
    let source = &data::wire(&state.settings[crate::timeline_easing::STORAGE])
        [pm_services::surfaces::easing::STORAGE_KEY];
    assert_eq!(source["groups"].as_array().unwrap().len(), 3);
    let destination = source["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["id"] == "destination")
        .unwrap();
    assert_eq!(destination["presets"][0]["id"], "copy");
    assert_eq!(destination["presets"][0]["color"], "#aabbcc");
}

/// v1–3's `showLayerNumbers` (and v4 `timeline.chrome`): numbers show until
/// the workspace says false, and come back when the extension leaves.
#[test]
fn layer_numbers_follow_the_workspace_chrome() {
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("timeline").unwrap());
    // The app republishes its panels on each kernel publication.
    let republish = |panels: &Builtins| {
        for (_, ticket) in panels.publish() {
            finish(ticket.unwrap());
        }
    };
    let numbered = |panels: &Builtins, want: bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let shown = panels
                .take_publications()
                .into_iter()
                .rev()
                .find(|p| p.panel == "timeline")
                .map(|p| p.surface.nodes().values().any(|n| n.text == "01"));
            if shown == Some(want) {
                return;
            }
            assert!(Instant::now() < deadline, "layer number shown != {want}");
            thread::sleep(Duration::from_millis(2));
        }
    };
    numbered(&panels, true);
    let graph = graph_of(
        "export function activate({ timeline }) { timeline.chrome({ showLayerNumbers: false }); }
        export const view = () => jsx('text', { children: 'Numbers off' });",
    );
    finish(panels.mount_extension("numbers-off", &graph, None).unwrap());
    chrome_when(&actor, |c| c["timeline"]["showLayerNumbers"] == false);
    republish(&panels);
    numbered(&panels, false);
    panels.unmount_extension("numbers-off");
    chrome_when(&actor, |c| c["timeline"].get("showLayerNumbers").is_none());
    republish(&panels);
    numbered(&panels, true);
    panels.close();
    actor.shutdown();
}

/// v1–3's `timeline.graph = shown` (the compat service sends this intent):
/// the timeline panel's view is shown or hidden as asked, never toggled, and
/// a combined view stays combined.
#[test]
fn the_graph_intent_sets_the_panels_view_as_asked() {
    use pm_panel_timeline::GraphDisplay::{Combined, DopeSheet, Graph};
    let actor = actor();
    let mut panels = Builtins::new(actor.handle(), "synthetic", || {}).unwrap();
    finish(panels.mount("timeline").unwrap());
    let view = |panels: &Builtins| {
        let entry = panels.broker.entry("timeline").unwrap();
        let entry = entry.lock().unwrap();
        let backend = entry.backend.lock().unwrap();
        backend.native.timeline.as_ref().unwrap().model.graph_display
    };
    let ask = |intent: Value| {
        finish(panels.timeline_action(intent).unwrap());
        view(&panels)
    };
    assert_eq!(ask(json!({"type":"graph","shown":true})), Graph);
    assert_eq!(ask(json!({"type":"graph","shown":true})), Graph);
    assert_eq!(ask(json!({"type":"graph","shown":false})), DopeSheet);
    assert_eq!(ask(json!({"type":"graph","shown":false})), DopeSheet);
    assert_eq!(ask(json!({"type":"view","view":"Combined"})), Combined);
    assert_eq!(ask(json!({"type":"graph","shown":true})), Combined);
}
