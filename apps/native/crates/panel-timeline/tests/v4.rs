use pm_kernel::{Caller, KernelActor, KernelConfig, OpenSource, SelectionCall};
use pm_model::json::parse;
use pm_panel_timeline::{v4, *};
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        std::thread::park_timeout(Duration::from_secs(1));
    }
}
fn client() -> (KernelActor, TimelineClient) {
    let value = parse(r#"{"id":"P","compId":"C","w":100,"h":100,"fps":30,"dur":10,"work":[0,10],"comps":{},"assets":{},"layers":[
    {"id":"a:[]","name":"Headline","type":"shape","from":-0.125,"dur":4,"on":true,"p":{"opacity":{"v":10,"kf":[{"i":"original","t":0.125,"v":10,"ei":[1,1],"eo":[0,0],"custom":"retained"}]}}},
    {"id":"b","name":"Kicker","type":"shape","from":5,"dur":2,"on":true,"p":{}}
    ]}"#).unwrap();
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    block_on(kernel.open("P", OpenSource::Untitled(value)))
        .unwrap()
        .unwrap();
    let client = TimelineClient::new(kernel, "P", Caller::native()).unwrap();
    (actor, client)
}
fn dispatch(c: &mut TimelineClient, value: Value) -> Result<Vec<HostEffect>, v4::Error> {
    let facts = PropertyFacts::default();
    let context = v4::Context {
        time: 0.,
        expected_document: c.model.index.doc_id,
        expected_revision: c.model.index.revision,
        millis: 100,
        in_field: false,
        keys_admitted: true,
        property_facts: &facts,
    };
    block_on(v4::dispatch(c, &value, &context))
}
fn id(id: &str) -> String {
    v4::row_id(&RowId::Layer(id.into()))
}

#[test]
fn stable_ids_roundtrip_arbitrary_imported_punctuation_without_collisions() {
    let rows = [
        RowId::Layer("a:[]\"🙂".into()),
        RowId::Property {
            layer_id: "a:[]\"🙂".into(),
            channel: "fx.layer:opacity".into(),
        },
    ];
    for row in rows {
        assert_eq!(v4::parse_row(&v4::row_id(&row)).unwrap(), row);
    }
    for bad in [
        "layer:a",
        "[]",
        "[1]",
        "[\"layer\",\"\"]",
        "[\"property\",\"a\"]",
    ] {
        assert!(v4::parse_row(bad).is_err());
    }
}
#[test]
fn selection_uses_actual_model_modifiers_and_preserves_document_revision() {
    let (_actor, mut c) = client();
    let revision = c.model.index.revision;
    dispatch(
        &mut c,
        json!({"type":"selectRow","id":id("a:[]"),"modifiers":{}}),
    )
    .unwrap();
    dispatch(
        &mut c,
        json!({"type":"selectRow","id":id("b"),"modifiers":{"shiftKey":true}}),
    )
    .unwrap();
    assert_eq!(c.model.selection.layers(), &["a:[]", "b"]);
    assert_eq!(c.model.index.revision, revision);
    dispatch(
        &mut c,
        json!({"type":"selectRow","id":id("a:[]"),"modifiers":{"metaKey":true}}),
    )
    .unwrap();
    assert_eq!(c.model.selection.layers(), &["b"]);
}
#[test]
fn tracks_preserve_the_models_toggle_instead_of_layers_range_selection() {
    let (_actor, mut c) = client();
    dispatch(&mut c, json!({"type":"mode","mode":"Tracks"})).unwrap();
    dispatch(
        &mut c,
        json!({"type":"selectRow","id":id("a:[]"),"modifiers":{}}),
    )
    .unwrap();
    dispatch(
        &mut c,
        json!({"type":"selectRow","id":id("a:[]"),"modifiers":{"shiftKey":true}}),
    )
    .unwrap();
    assert!(c.model.selection.layers().is_empty());
    assert!(matches!(
        v4::window(&mut c, 0, 10),
        Err(v4::Error::MissingHost("track lane presentation"))
    ));
}
#[test]
fn modes_view_and_snap_publish_ui_without_document_history_or_dirtying() {
    let (_actor, mut c) = client();
    let revision = c.model.index.revision;
    for (kind, name, value) in [("mode", "mode", "Tracks"), ("view", "view", "Combined")] {
        let mut intent = json!({"type":kind});
        intent[name] = json!(value);
        dispatch(&mut c, intent).unwrap();
    }
    dispatch(&mut c, json!({"type":"snap"})).unwrap();
    let state = c.kernel.state();
    let ui = &state.ui["P"].timeline;
    assert_eq!(
        ui.get("mode").and_then(pm_model::Value::as_str),
        Some("tracks")
    );
    assert_eq!(
        ui.get("graphCombined").and_then(pm_model::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        ui.get("trackSnap").and_then(pm_model::Value::as_bool),
        Some(false)
    );
    assert_eq!(c.model.index.revision, revision);
}
#[test]
fn disclosure_is_idempotent_for_explicit_open_and_uses_native_row_index() {
    let (_actor, mut c) = client();
    for _ in 0..2 {
        dispatch(
            &mut c,
            json!({"type":"disclosure","id":id("a:[]"),"open":true}),
        )
        .unwrap();
    }
    let result = v4::window(&mut c, 0, 10).unwrap();
    assert_eq!(result["window"]["rows"][0]["expanded"], true);
    assert_eq!(result["window"]["rows"][0]["index"], 1);
    assert_eq!(result["window"]["rows"][1]["path"], "opacity");
    assert_eq!(c.model.index.layer_ordinal("b"), Some(1));
}
#[test]
fn visibility_switch_is_a_real_kernel_edit_with_undo() {
    let (_actor, mut c) = client();
    dispatch(
        &mut c,
        json!({"type":"switch","id":id("a:[]"),"name":"visibility"}),
    )
    .unwrap();
    assert!(!c.model.index.layer("a:[]").unwrap().visible);
    block_on(c.call(pm_kernel::HistoryCall::Undo.into())).unwrap();
    assert!(c.model.index.layer("a:[]").unwrap().visible);
}
#[test]
fn diamond_uses_original_key_identity_and_native_undo_retains_opaque_fields() {
    let (_actor, mut c) = client();
    dispatch(
        &mut c,
        json!({"type":"keyToggle","layer":"a:[]","path":"opacity"}),
    )
    .unwrap();
    assert!(c.model.index.key("original").is_none());
    block_on(c.call(pm_kernel::HistoryCall::Undo.into())).unwrap();
    assert_eq!(
        c.model
            .index
            .key("original")
            .unwrap()
            .2
            .stored
            .get("custom")
            .and_then(pm_model::Value::as_str),
        Some("retained")
    );
}
#[test]
fn shortcuts_require_shell_admission_and_preserve_exact_repeat_policy() {
    let (_actor, mut c) = client();
    assert_eq!(
        dispatch(&mut c, json!({"type":"keyDown","event":{"key":"PageDown"}})).unwrap(),
        [HostEffect::Seek(1. / 30.)]
    );
    assert_eq!(dispatch(&mut c, json!({"type":"keyDown","event":{"key":"ArrowRight","metaKey":true,"shiftKey":true,"repeat":true}})).unwrap(), [HostEffect::Seek(10. / 30.)]);
    assert!(
        dispatch(
            &mut c,
            json!({"type":"keyDown","event":{"key":" ","repeat":true}})
        )
        .is_err()
    );
    let facts = PropertyFacts::default();
    let context = v4::Context {
        time: 0.,
        expected_document: c.model.index.doc_id,
        expected_revision: c.model.index.revision,
        millis: 0,
        in_field: false,
        keys_admitted: false,
        property_facts: &facts,
    };
    assert!(matches!(
        block_on(v4::dispatch(
            &mut c,
            &json!({"type":"keyDown","event":{"key":" "}}),
            &context
        )),
        Err(v4::Error::MissingHost("shell shortcut arbitration"))
    ));
}
#[test]
fn host_effects_are_requests_not_transport_or_import_success() {
    let (_actor, mut c) = client();
    assert_eq!(
        dispatch(&mut c, json!({"type":"play"})).unwrap(),
        [HostEffect::TogglePlayback]
    );
    assert!(
        matches!(&dispatch(&mut c, json!({"type":"import"})).unwrap()[0], HostEffect::Import { time, before_layer: None, as_folder: false } if *time == 0.)
    );
    for kind in [
        "graphOptions",
        "open",
        "tab",
        "timeEdit",
        "rename",
        "pointerDown",
        "drop",
    ] {
        assert!(matches!(
            dispatch(&mut c, json!({"type":kind})),
            Err(v4::Error::MissingHost(_))
        ));
    }
}
#[test]
fn stale_surface_and_malformed_requests_fail_before_any_edit() {
    let (_actor, mut c) = client();
    let revision = c.model.index.revision;
    let facts = PropertyFacts::default();
    let context = v4::Context {
        time: 0.,
        expected_document: c.model.index.doc_id + 1,
        expected_revision: revision,
        millis: 0,
        in_field: false,
        keys_admitted: true,
        property_facts: &facts,
    };
    assert!(
        block_on(v4::dispatch(
            &mut c,
            &json!({"type":"switch","id":id("a:[]"),"name":"visibility"}),
            &context
        ))
        .is_err()
    );
    for value in [
        json!({"type":"unknown"}),
        json!({"type":"mode","mode":"other"}),
        json!({"type":"selectRow","id":id("b"),"modifiers":{"shiftKey":"true"}}),
    ] {
        assert!(dispatch(&mut c, value).is_err());
    }
    assert_eq!(c.model.index.revision, revision);
    assert!(c.model.index.layer("a:[]").unwrap().visible);
}
#[test]
fn window_keeps_exact_negative_subframes_and_decimal_stamps() {
    let (_actor, mut c) = client();
    let result = v4::window(&mut c, 0, 1).unwrap();
    assert!(result["document"].is_string());
    assert!(result["revision"].is_string());
    assert_eq!(result["window"]["rows"][0]["from"], -0.125);
    assert_eq!(result["window"]["total"], 2);
    assert_eq!(result["window"]["rows"].as_array().unwrap().len(), 2);
    assert!(v4::window(&mut c, 0, 0).is_err());
}
#[test]
fn selection_facts_in_window_follow_the_actual_actor_publication() {
    let (_actor, mut c) = client();
    block_on(
        c.call(
            SelectionCall::Layers {
                ids: vec!["b".into()],
                add: false,
            }
            .into(),
        ),
    )
    .unwrap();
    let result = v4::window(&mut c, 0, 10).unwrap();
    assert_eq!(result["selection"], json!({"kind":"layers","count":1}));
    assert_eq!(result["window"]["rows"][0]["selected"], false);
    assert_eq!(result["window"]["rows"][1]["selected"], true);
}

#[test]
fn sixty_thousand_real_model_rows_project_only_three_viewports() {
    let mut project = json!({"id":"P","compId":"C","w":100,"h":100,"fps":30,"dur":10,"work":[0,10],"comps":{},"assets":{}});
    project["layers"] = Value::Array((0..60_000).map(|i| json!({"id":format!("row-{i}"),"name":"Headline","type":"shape","from":0,"dur":1,"on":true,"p":{}})).collect());
    let actor = KernelActor::spawn(KernelConfig::default());
    let kernel = actor.handle();
    block_on(kernel.open(
        "P",
        OpenSource::Untitled(parse(&project.to_string()).unwrap()),
    ))
    .unwrap()
    .unwrap();
    let mut c = TimelineClient::new(kernel, "P", Caller::native()).unwrap();
    let value = v4::window(&mut c, 30_000, 10).unwrap();
    assert_eq!(value["window"]["total"], 60_000);
    assert_eq!(value["window"]["first"], 29_990);
    assert_eq!(value["window"]["rows"].as_array().unwrap().len(), 30);
    assert_eq!(value["window"]["rows"][0]["index"], 29_991);
    assert_eq!(value["indexDigits"], 5);
}
