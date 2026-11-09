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
    let frame = v4::window(&mut c, 0, 10).unwrap();
    assert_eq!(frame["mode"], "Tracks");
    assert!(
        frame["window"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["kind"] == "track")
    );
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
    assert!(matches!(
        dispatch(&mut c, json!({"type":"tab","id":"c","close":true})),
        Err(v4::Error::MissingHost(_))
    ));
    assert_eq!(
        dispatch(&mut c, json!({"type":"tab","id":"c","close":false})).unwrap(),
        [HostEffect::OpenComposition("c".into())]
    );
    assert_eq!(
        dispatch(&mut c, json!({"type":"seek","time":1.5})).unwrap(),
        [HostEffect::Seek(1.5)]
    );
    // Pointer gestures run on the native surface; JS cannot start one.
    for kind in ["pointerDown", "drop"] {
        assert!(matches!(
            dispatch(&mut c, json!({"type":kind})),
            Err(v4::Error::InvalidIntent(_))
        ));
    }
}
#[test]
fn typed_timecode_takes_any_leading_fields_or_bare_frames() {
    assert_eq!(v4::parse_timecode("00:00:02:15", 30.), Some(2.5));
    assert_eq!(v4::parse_timecode("2:15", 30.), Some(2.5));
    assert_eq!(v4::parse_timecode("75", 30.), Some(2.5));
    assert_eq!(v4::parse_timecode("1:00:00", 30.), Some(60.));
    assert_eq!(v4::parse_timecode("nonsense", 30.), None);
    assert_eq!(v4::parse_timecode("", 30.), None);
    // 29.97 shows frame 18000 as 00:10:00:00; typing it back lands there.
    let fps = 30000. / 1001.;
    let time = v4::parse_timecode("00:10:00:00", fps).unwrap();
    assert_eq!((time * fps).round(), 18000.);
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

#[test]
fn property_navigation_uses_model_composition_times_and_does_not_wrap() {
    let (_actor, mut c) = client();
    let facts = PropertyFacts::default();
    let context = v4::Context {
        time: 0.5,
        expected_document: c.model.index.doc_id,
        expected_revision: c.model.index.revision,
        millis: 0,
        in_field: false,
        keys_admitted: true,
        property_facts: &facts,
    };
    let intent =
        json!({"type":"adjacentKey","layer":"a:[]","path":"opacity","direction":"previous"});
    assert_eq!(
        block_on(v4::dispatch(&mut c, &intent, &context)).unwrap(),
        [HostEffect::Seek(0.)]
    );
    assert!(
        dispatch(
            &mut c,
            json!({"type":"adjacentKey","layer":"a:[]","path":"opacity","direction":"next"})
        )
        .unwrap()
        .is_empty()
    );
}
#[test]
fn scale_link_is_undoable_through_the_actual_layer_command() {
    let (_actor, mut c) = client();
    dispatch(&mut c, json!({"type":"scaleLink","layer":"a:[]"})).unwrap();
    assert!(c.model.index.layer("a:[]").unwrap().scale_linked);
    block_on(c.call(pm_kernel::HistoryCall::Undo.into())).unwrap();
    assert!(!c.model.index.layer("a:[]").unwrap().scale_linked);
}
#[test]
fn escape_cancels_an_active_native_preview_before_deselecting() {
    let (_actor, mut c) = client();
    let original = c.model.index.layer("a:[]").unwrap().from;
    block_on(c.begin_drag(DragKind::Layers {
        ids: vec!["a:[]".into()],
    }))
    .unwrap();
    block_on(c.drag(DragSample {
        time_delta: 1.,
        ..Default::default()
    }))
    .unwrap();
    assert_ne!(c.model.index.layer("a:[]").unwrap().from, original);
    dispatch(&mut c, json!({"type":"keyDown","event":{"key":"Escape"}})).unwrap();
    assert_eq!(c.model.index.layer("a:[]").unwrap().from, original);
    assert!(c.active_drag().is_none());
}
#[test]
fn window_refreshes_the_actual_kernel_publication_without_an_intervening_client_action() {
    let (_actor, mut c) = client();
    block_on(c.kernel.call(
        "P",
        pm_kernel::Request::new(
            Caller::native(),
            SelectionCall::Layers {
                ids: vec!["b".into()],
                add: false,
            },
        ),
    ))
    .unwrap()
    .unwrap();
    assert!(c.model.selection.layers().is_empty());
    let result = v4::window(&mut c, 0, 10).unwrap();
    assert_eq!(result["window"]["rows"][1]["selected"], true);
}

#[test]
fn renderer_adopts_actual_model_timing_selection_and_document_rows() {
    use pm_design::{Accessibility, Scheme, Tokens};
    use pm_panel_timeline::render as paint;
    struct Presentation;
    impl paint::ModelPresenter for Presentation {
        fn accessibility_label(&self, row: &RowId) -> String {
            v4::row_id(row)
        }
        fn layer_label(&self, _: &Layer) -> paint::Label {
            paint::Label::Blue
        }
        fn media(&self, _: &Layer) -> paint::Media {
            paint::Media::Ready
        }
        fn waveform(&self, _: &Layer) -> Option<Option<paint::Waveform>> {
            Some(Some(paint::Waveform {
                peaks: vec![0., 0.25, 0.5].into(),
                rate: 100.,
                trim: 0.,
            }))
        }
        fn interpolation(&self, _: &Key) -> paint::Interpolation {
            paint::Interpolation::Linear
        }
        fn track_accessibility_label(&self, area: tracks::Area, lane: usize) -> Option<String> {
            Some(format!("{area:?} {}", lane + 1))
        }
    }
    let (_actor, mut c) = client();
    dispatch(
        &mut c,
        json!({"type":"selectRow","id":id("a:[]"),"modifiers":{}}),
    )
    .unwrap();
    let t = Tokens::new(Scheme::Dark, Accessibility::default());
    let scene = paint::Scene::from_model(&c.model, &Presentation, &t, 0.).unwrap();
    assert_eq!(scene.total(), c.model.rows().len());
    let row = scene.row(&id("a:[]")).unwrap();
    assert!(row.selected);
    let paint::RowContent::Layer {
        clip: Some(clip),
        summary,
    } = &row.content
    else {
        panic!("layer paint");
    };
    assert_eq!(clip.from, -0.125);
    assert_eq!(clip.duration, 4.);
    assert_eq!(
        clip.audio
            .as_ref()
            .and_then(|w| w.as_ref())
            .map(|w| w.peaks.len()),
        Some(3)
    );
    assert_eq!(summary[0].seconds, 0.);
    c.model.mode = TimelineMode::Tracks;
    let scene = paint::Scene::from_model(&c.model, &Presentation, &t, 0.).unwrap();
    assert!(
        matches!(&scene.row("[\"track\",\"Video\",0]").unwrap().content,paint::RowContent::Track{clips} if clips.len()==2)
    );
}

#[test]
fn shell_mask_reveal_persists_disclosure_without_editing_animation() {
    let (_actor, mut c) = client();
    let before = c.kernel.published("P").unwrap();
    dispatch(&mut c, json!({"type":"keyDown","event":{"key":"m"}})).unwrap();
    let shown = c.model.reveal.layers.clone();
    assert!(!shown.is_empty());
    assert!(
        shown
            .values()
            .all(|d| !d.collapsed && d.paths == Some(vec!["*".into()]))
    );
    let after = c.kernel.published("P").unwrap();
    assert_eq!(after.revision(), before.revision());
    assert_eq!(
        after.snapshot.document.to_value(),
        before.snapshot.document.to_value()
    );
    c.sync().unwrap();
    assert_eq!(c.model.reveal.layers, shown);
    dispatch(&mut c, json!({"type":"keyDown","event":{"key":"m"}})).unwrap();
    assert!(c.model.reveal.layers.values().all(|d| d.collapsed));
}

#[test]
fn native_drag_steps_replace_from_start_and_commit_as_one_undo() {
    let (_actor, mut c) = client();
    let before = c.model.index.layer("a:[]").unwrap().from;
    dispatch(
        &mut c,
        json!({"type":"beginDrag","kind":"layers","ids":["a:[]"]}),
    )
    .unwrap();
    dispatch(&mut c, json!({"type":"drag","timeDelta":1.})).unwrap();
    dispatch(&mut c, json!({"type":"drag","timeDelta":2.})).unwrap();
    let moved = c.model.index.layer("a:[]").unwrap().from;
    // Every sample is total displacement, not a repeated relative increment.
    assert!((moved - before - 2.).abs() < 1. / 30.);
    dispatch(&mut c, json!({"type":"endDrag"})).unwrap();
    block_on(c.call(pm_kernel::HistoryCall::Undo.into())).unwrap();
    assert_eq!(c.model.index.layer("a:[]").unwrap().from, before);
}

#[test]
fn native_disposal_and_pointer_cancellation_restore_the_active_preview() {
    for action in ["cancelDrag", "dispose"] {
        let (_actor, mut c) = client();
        let before = c.model.index.layer("b").unwrap().duration;
        dispatch(
            &mut c,
            json!({"type":"beginDrag","kind":"trimOut","ids":["b"]}),
        )
        .unwrap();
        dispatch(&mut c, json!({"type":"drag","timeDelta":1.})).unwrap();
        assert_ne!(c.model.index.layer("b").unwrap().duration, before);
        dispatch(&mut c, json!({"type":action})).unwrap();
        assert_eq!(c.model.index.layer("b").unwrap().duration, before);
        assert!(c.active_drag().is_none());
    }
}

#[test]
fn numeric_key_time_uses_composition_time_beyond_strip_and_refuses_collision() {
    let (_actor, mut c) = client();
    dispatch(
        &mut c,
        json!({"type":"editKeys","ids":["original"],"time":8.,"value":42.}),
    )
    .unwrap();
    let (l, _, k) = c.model.index.key("original").unwrap();
    // main timeline.ts snaps the layer-local time, retaining a subframe strip origin.
    assert_eq!(k.time, ((8.0_f64 + 0.125) * 30.).round() / 30.);
    assert!(l.from + k.time > l.from + l.duration);
    assert_eq!(k.value.as_f64(), Some(42.));
    assert_eq!(
        k.stored.get("custom").and_then(pm_model::Value::as_str),
        Some("retained")
    );
    dispatch(
        &mut c,
        json!({"type":"keyToggle","layer":"a:[]","path":"opacity"}),
    )
    .unwrap();
    let revision = c.model.index.revision;
    assert!(
        dispatch(
            &mut c,
            json!({"type":"editKeys","ids":["original"],"time":0.})
        )
        .is_err()
    );
    assert_eq!(c.model.index.revision, revision);
    assert_eq!(
        c.model.index.key("original").unwrap().2.value.as_f64(),
        Some(42.)
    );
}

fn position_client() -> (KernelActor, TimelineClient) {
    let value = parse(r#"{"id":"P","compId":"C","w":100,"h":100,"fps":30,"dur":10,"work":[0,10],"comps":{},"assets":{},"layers":[{"id":"p","type":"shape","name":"Position","collapsed":false,"from":0,"dur":10,"p":{"position.x":{"v":0,"kf":[{"i":"x","t":0.125,"v":1,"custom":"keep"}]},"position.y":{"v":0,"kf":[{"i":"y","t":0.125,"v":2},{"i":"subframe","t":0.125001,"v":3}]}}}]}"#).unwrap();
    let actor = KernelActor::spawn(KernelConfig::default());
    block_on(actor.handle().open("P", OpenSource::Untitled(value)))
        .unwrap()
        .unwrap();
    let c = TimelineClient::new(actor.handle(), "P", Caller::native()).unwrap();
    (actor, c)
}

#[test]
fn separate_dimensions_is_undoable_and_groups_only_exact_legacy_key_times() {
    let (_actor, mut c) = position_client();
    assert!(c.model.index.layer("p").unwrap().separate_dimensions);
    dispatch(
        &mut c,
        json!({"type":"separateDimensions","layer":"p","separate":false}),
    )
    .unwrap();
    assert!(!c.model.index.layer("p").unwrap().separate_dimensions);
    assert!(
        c.model
            .rows()
            .iter()
            .any(|r| matches!(&r.id,RowId::Property { channel,.. } if channel == "position"))
    );
    dispatch(
        &mut c,
        json!({"type":"selectKeys","layer":"p","path":"position","ids":["x"]}),
    )
    .unwrap();
    assert_eq!(c.model.selection.keys(), &["x", "y"]);
    assert_eq!(c.model.index.key("subframe").unwrap().2.time, 0.125001);
    block_on(c.call(pm_kernel::HistoryCall::Undo.into())).unwrap();
    assert!(c.model.index.layer("p").unwrap().separate_dimensions);
    assert!(
        c.model
            .rows()
            .iter()
            .any(|r| matches!(&r.id,RowId::Property { channel,.. } if channel == "position.x"))
    );
    assert!(
        !c.model
            .rows()
            .iter()
            .any(|r| matches!(&r.id,RowId::Property { channel,.. } if channel == "position"))
    );
}

#[test]
fn axis_diamond_changes_only_its_axis_and_group_diamond_completes_the_other_axis() {
    let (_actor, mut c) = position_client();
    dispatch(
        &mut c,
        json!({"type":"separateDimensions","layer":"p","separate":false}),
    )
    .unwrap();
    dispatch(
        &mut c,
        json!({"type":"keyToggle","layer":"p","path":"position.x"}),
    )
    .unwrap();
    assert_eq!(
        c.model.index.channel("p", "position.x").unwrap().keys.len(),
        2
    );
    assert_eq!(
        c.model.index.channel("p", "position.y").unwrap().keys.len(),
        2
    );
    dispatch(
        &mut c,
        json!({"type":"keyToggle","layer":"p","path":"position"}),
    )
    .unwrap();
    assert_eq!(
        c.model.index.channel("p", "position.x").unwrap().keys.len(),
        2
    );
    assert_eq!(
        c.model.index.channel("p", "position.y").unwrap().keys.len(),
        3
    );
}

#[test]
fn native_graph_drag_and_easing_keep_ids_and_opaque_metadata_through_undo() {
    let (_actor, mut c) = client();
    dispatch(
        &mut c,
        json!({"type":"beginDrag","kind":"graphKeys","ids":["original"]}),
    )
    .unwrap();
    dispatch(
        &mut c,
        json!({"type":"drag","timeDelta":1.,"valueDelta":3.}),
    )
    .unwrap();
    dispatch(&mut c, json!({"type":"endDrag"})).unwrap();
    assert_eq!(
        c.model.index.key("original").unwrap().2.value.as_f64(),
        Some(13.)
    );
    dispatch(
        &mut c,
        json!({"type":"easing","ids":["original"],"curve":[0.25,0.1,0.25,1.]}),
    )
    .unwrap();
    dispatch(&mut c,json!({"type":"interpolation","ids":["original"],"side":"both","interpolation":"continuous"})).unwrap();
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
    for _ in 0..3 {
        block_on(c.call(pm_kernel::HistoryCall::Undo.into())).unwrap();
    }
    assert_eq!(
        c.model.index.key("original").unwrap().2.value.as_f64(),
        Some(10.)
    );
    assert_eq!(c.model.index.key("original").unwrap().2.time, 0.125);
}

#[test]
fn native_parent_pickwhip_uses_kernel_cycle_validation_and_undo() {
    let (_actor, mut c) = client();
    dispatch(&mut c, json!({"type":"parent","ids":["b"],"parent":"a:[]"})).unwrap();
    assert_eq!(
        c.model.index.layer("b").unwrap().parent.as_deref(),
        Some("a:[]")
    );
    let revision = c.model.index.revision;
    assert!(dispatch(&mut c, json!({"type":"parent","ids":["a:[]"],"parent":"b"})).is_err());
    assert_eq!(c.model.index.revision, revision);
    block_on(c.call(pm_kernel::HistoryCall::Undo.into())).unwrap();
    assert!(c.model.index.layer("b").unwrap().parent.is_none());
}

#[test]
fn malformed_native_actions_and_stale_decimal_stamps_make_no_edit_or_lease() {
    let (_actor, mut c) = client();
    let revision = c.model.index.revision;
    for intent in [
        json!({"type":"beginDrag","kind":"layers","ids":["b","b"]}),
        json!({"type":"beginDrag","kind":"layers","ids":[]}),
        json!({"type":"beginDrag","kind":"layers","ids":["missing"]}),
        json!({"type":"beginDrag","kind":"layers","ids":["b"],"document":"9999999999999999999"}),
        json!({"type":"editKeys","ids":["original"],"time":"bad"}),
        json!({"type":"easing","ids":["original"],"curve":[1,2,3]}),
        json!({"type":"selectKeys","layer":"b","path":"opacity","ids":["original"]}),
    ] {
        assert!(dispatch(&mut c, intent).is_err());
    }
    assert_eq!(c.model.index.revision, revision);
    assert!(c.active_drag().is_none());
}

fn media_client() -> (KernelActor, TimelineClient) {
    let layers = [("a",0.),("b",4.),("c",8.)].into_iter().map(|(id,from)|json!({"id":id,"name":id,"type":"video","from":from,"dur":4,"track":0,"on":true,"d":{"asset":"source","trim":{"v":from+2.,"kf":[],"expr":null},"speed":1,"future":{"keep":true}},"p":{},"future":{"keep":id}})).collect::<Vec<_>>();
    let value = json!({"id":"P","compId":"C","fps":30,"dur":40,"w":100,"h":100,"assets":{"source":{"id":"source","kind":"video","dur":100}},"layers":layers});
    let actor = KernelActor::spawn(KernelConfig::default());
    block_on(actor.handle().open(
        "P",
        OpenSource::Untitled(parse(&value.to_string()).unwrap()),
    ))
    .unwrap()
    .unwrap();
    let c = TimelineClient::new(actor.handle(), "P", Caller::native()).unwrap();
    (actor, c)
}
fn document(c: &TimelineClient) -> pm_model::Value {
    c.kernel
        .published(&c.project)
        .unwrap()
        .snapshot
        .root()
        .unwrap()
        .to_value()
}
#[test]
fn all_scoped_pro_edit_actions_reach_the_real_kernel_and_undo_exactly() {
    for (mut input, target, from) in [
        (json!({"operation":"move","ids":["b"],"delta":1.}), "b", 5.),
        (
            json!({"operation":"trim","edge":"in","ids":["b"],"delta":1.}),
            "b",
            5.,
        ),
        (json!({"operation":"slip","ids":["b"],"delta":1.}), "b", 4.),
        (
            json!({"operation":"rippleTrim","edge":"out","layer":"b","delta":1.,"tracks":[{"track":0}]}),
            "c",
            9.,
        ),
        (
            json!({"operation":"roll","left":"a","right":"b","delta":1.}),
            "b",
            5.,
        ),
        (
            json!({"operation":"slide","previous":"a","layer":"b","next":"c","delta":1.}),
            "b",
            5.,
        ),
        (
            json!({"operation":"rippleDelete","ids":["b"],"tracks":[{"track":0}]}),
            "c",
            4.,
        ),
    ] {
        let (_actor, mut c) = media_client();
        let before = document(&c);
        input["type"] = json!("proEdit");
        dispatch(&mut c, input).unwrap();
        assert_eq!(c.model.index.layer(target).unwrap().from, from);
        assert_ne!(document(&c), before);
        block_on(c.call(pm_kernel::HistoryCall::Undo.into())).unwrap();
        assert_eq!(document(&c), before);
    }
}

#[test]
fn ripple_pointer_steps_use_total_delta_and_dispose_restores_downstream_clips() {
    let (_actor, mut c) = media_client();
    let before = document(&c);
    dispatch(&mut c,json!({"type":"beginDrag","kind":"proEdit","operation":"rippleTrim","layer":"b","edge":"out","delta":0.,"tracks":[{"track":0}]})).unwrap();
    dispatch(&mut c, json!({"type":"drag","timeDelta":1.})).unwrap();
    dispatch(&mut c, json!({"type":"drag","timeDelta":2.})).unwrap();
    assert_eq!(c.model.index.layer("b").unwrap().duration, 6.);
    assert_eq!(c.model.index.layer("c").unwrap().from, 10.);
    dispatch(&mut c, json!({"type":"dispose"})).unwrap();
    assert_eq!(document(&c), before);
}

#[test]
fn locked_downstream_pro_drag_refuses_before_a_gesture_and_preserves_document() {
    let (_actor, mut c) = media_client();
    dispatch(&mut c, json!({"type":"switch","id":id("c"),"name":"lock"})).unwrap();
    let before = document(&c);
    assert!(dispatch(&mut c,json!({"type":"beginDrag","kind":"proEdit","operation":"rippleTrim","layer":"b","edge":"out","delta":1.,"tracks":[{"track":0}]})).is_err());
    assert!(c.active_drag().is_none());
    assert_eq!(document(&c), before);
}

#[test]
fn simplify_is_explicit_and_retains_unknown_authored_key_fields() {
    let (_actor, mut c) = client();
    let before = document(&c);
    dispatch(&mut c, json!({"type":"simplify","ids":["a:[]"]})).unwrap();
    assert_eq!(
        document(&c),
        before,
        "unknown authored key fields prevent simplification"
    );
}

#[test]
fn old_native_gesture_cannot_edit_a_reopened_document_with_reused_layer_ids() {
    let (_actor, mut c) = media_client();
    dispatch(
        &mut c,
        json!({"type":"beginDrag","kind":"layers","ids":["b"]}),
    )
    .unwrap();
    let saved = document(&c);
    block_on(c.kernel.close("P")).unwrap();
    block_on(c.kernel.open("P", OpenSource::Untitled(saved.clone())))
        .unwrap()
        .unwrap();
    c.sync().unwrap();
    assert!(matches!(
        dispatch(&mut c, json!({"type":"drag","timeDelta":2.})),
        Err(v4::Error::Client(ClientError::DocumentChanged))
    ));
    assert!(matches!(
        dispatch(&mut c, json!({"type":"dispose"})),
        Err(v4::Error::Client(ClientError::DocumentChanged))
    ));
    assert!(c.active_drag().is_none());
    assert_eq!(document(&c), saved);
}

#[test]
fn handle_property_and_work_area_native_gestures_cancel_without_losing_authored_data() {
    for begin in [
        json!({"type":"beginDrag","kind":"handle","id":"original","side":"out"}),
        json!({"type":"beginDrag","kind":"property","layer":"a:[]","path":"opacity"}),
        json!({"type":"beginDrag","kind":"workIn"}),
        json!({"type":"beginDrag","kind":"workOut"}),
        json!({"type":"beginDrag","kind":"workMove"}),
        json!({"type":"beginDrag","kind":"transformKeys","ids":["original"],"anchorTime":0.,"anchorValue":0.}),
        json!({"type":"beginDrag","kind":"stretchKeys","ids":["original"],"anchorTime":0.,"grabbedTime":1.}),
        json!({"type":"beginDrag","kind":"keys","ids":["original"]}),
        json!({"type":"beginDrag","kind":"trimIn","ids":["a:[]"]}),
        json!({"type":"beginDrag","kind":"offsetLayers","ids":["a:[]","b"]}),
    ] {
        let (_actor, mut c) = client();
        let before = document(&c);
        dispatch(&mut c, begin).unwrap();
        dispatch(&mut c,json!({"type":"drag","timeDelta":0.1,"valueDelta":1.,"timeScale":1.5,"valueScale":2.,"handleTime":0.25,"handleValue":0.5})).unwrap();
        dispatch(&mut c, json!({"type":"cancelDrag"})).unwrap();
        assert_eq!(document(&c), before);
    }
}

#[test]
fn native_numeric_key_dialog_rejects_non_numeric_values_before_any_edit() {
    let (_actor, mut c) = client();
    let before = document(&c);
    for value in [json!(null), json!("12"), json!([1, 2]), json!({"v":12})] {
        assert!(
            dispatch(
                &mut c,
                json!({"type":"editKeys","ids":["original"],"value":value})
            )
            .is_err()
        );
        assert_eq!(document(&c), before);
    }
}

#[test]
fn graph_native_commands_persist_view_facts_without_document_edits() {
    let (_actor, mut c) = client();
    let before = document(&c);
    dispatch(&mut c, json!({"type":"command","command":"graph.speed"})).unwrap();
    let state = c.kernel.state();
    assert_eq!(
        state.ui["P"]
            .timeline
            .get("graphMode")
            .and_then(pm_model::Value::as_str),
        Some("speed")
    );
    assert_eq!(document(&c), before);
}

#[test]
fn presentation_can_hold_one_immutable_publication_while_actor_advances() {
    let (_actor, mut c) = client();
    let old = c.model.clone();
    let revision = old.index.revision.to_string();
    dispatch(
        &mut c,
        json!({"type":"switch","id":id("b"),"name":"visibility"}),
    )
    .unwrap();
    let old_frame = v4::project_window(&old, 0, 10).unwrap();
    assert_eq!(old_frame["revision"], revision);
    let old_b = old_frame["window"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["layer"] == "b")
        .unwrap();
    assert_eq!(old_b["visible"], true);
    let current = v4::window(&mut c, 0, 10).unwrap();
    assert_ne!(current["revision"], old_frame["revision"]);
    let new_b = current["window"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["layer"] == "b")
        .unwrap();
    assert_eq!(new_b["visible"], false);
}
