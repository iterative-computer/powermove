use pm_kernel::commands::keyframes::{EaseSide, Interpolation};
use pm_kernel::{
    Call, Caller, Command, DocumentStore, HistoryCall, KernelActor, KernelConfig, KeyframeCommand,
    OpenSource, Origin, PropertyCommand, Reply, Request, SelectionCall, WriteBody,
};
use pm_model::{Value, json::parse};
use pm_panel_timeline::snap::{Anchor, DirectionalSnap, Edge, Target};
use pm_panel_timeline::*;
use std::{
    future::Future,
    pin::pin,
    sync::Arc,
    task::{Context as TaskContext, Poll, Wake, Waker},
    time::Duration,
};

const FIXTURE: &str = r##"{"id":"P","compId":"C","name":"Synthetic timeline","w":200,"h":100,"fps":30,"dur":10,"work":[1,9],"comps":{},"assets":{},"markers":[{"id":"marker","t":4,"name":"Authored"}],"layers":[
{"id":"a","name":"A","type":"shape","from":1,"dur":4,"on":true,"collapsed":false,"p":{"opacity":{"v":10,"kf":[{"i":"a0","t":0,"v":10,"eo":[0,0],"ei":[1,1],"hold":false,"custom":{"keep":true}},{"i":"a2","t":2,"v":50,"eo":[0,0],"ei":[1,1],"hold":false}],"expr":null},"position.x":{"v":0,"kf":[{"i":"px","t":1,"v":5,"eo":[0,0],"ei":[1,1]}],"expr":null},"scale.x":{"v":100,"kf":[{"i":"sx","t":0.125,"v":100,"eo":[0,0],"ei":[1,1]}],"expr":null},"scale.y":{"v":100,"kf":[{"i":"sy","t":0.125,"v":100,"eo":[0,0],"ei":[1,1]}],"expr":null}}},
{"id":"b","name":"B","type":"shape","from":5,"dur":2,"on":true,"p":{"opacity":{"v":80,"kf":[{"i":"b0","t":0,"v":80,"eo":[0,0],"ei":[1,1]}],"expr":null}}},
{"id":"c","name":"C","type":"shape","from":-2,"dur":3,"on":true,"p":{"opacity":{"v":70,"kf":[],"expr":null}}}
]}"##;
fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(std::thread::Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = TaskContext::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::park_timeout(Duration::from_secs(1));
    }
}
fn model(value: Value) -> TimelineModel {
    let store = DocumentStore::new_untitled(value);
    let mut m = TimelineModel::default();
    m.index = Arc::new(TimelineIndex::from_snapshot(&store.snapshot()));
    m.rebuild_rows();
    m
}
fn fixture() -> Value {
    parse(FIXTURE).unwrap()
}
fn client_with(value: Value) -> (KernelActor, TimelineClient) {
    let actor = KernelActor::spawn(KernelConfig::default());
    let k = actor.handle();
    block_on(k.open("P", OpenSource::Untitled(value)))
        .unwrap()
        .unwrap();
    let c = TimelineClient::new(k, "P", Caller::native()).unwrap();
    (actor, c)
}
fn client() -> (KernelActor, TimelineClient) {
    client_with(fixture())
}
fn run(c: &mut TimelineClient, p: Plan) -> Vec<HostEffect> {
    block_on(c.dispatch(p)).unwrap()
}
fn command(c: &mut TimelineClient, id: &str, time: f64) -> Vec<HostEffect> {
    let p = c
        .model
        .command(id, time, None, false, 0, &PropertyFacts::default())
        .unwrap();
    run(c, p)
}
fn select(c: &mut TimelineClient, ids: &[&str]) {
    block_on(
        c.call(
            SelectionCall::Layers {
                ids: ids.iter().map(|s| (*s).into()).collect(),
                add: false,
            }
            .into(),
        ),
    )
    .unwrap();
}
fn select_keys(c: &mut TimelineClient, ids: &[&str]) {
    block_on(c.call(SelectionCall::Keys(ids.iter().map(|s| (*s).into()).collect()).into()))
        .unwrap();
}
fn edit(c: &mut TimelineClient, commands: Vec<Command>) {
    let p = Plan::edit(commands, c.model.index.revision);
    run(c, p);
}
fn undo(c: &mut TimelineClient) {
    block_on(c.call(HistoryCall::Undo.into())).unwrap();
}
fn sample(delta: f64) -> DragSample {
    DragSample {
        time_delta: delta,
        playhead: 8.,
        time_scale: 1.,
        value_scale: 1.,
        ..Default::default()
    }
}
fn key_time(c: &TimelineClient, id: &str) -> f64 {
    c.model.index.key(id).unwrap().2.time
}
fn strings(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| (*s).into()).collect()
}
fn mods() -> Modifiers {
    Modifiers::default()
}
fn layer_mut(value: &mut Value, index: usize) -> &mut pm_model::Map {
    value
        .as_object_mut()
        .unwrap()
        .get_mut("layers")
        .unwrap()
        .as_array_mut()
        .unwrap()[index]
        .as_object_mut()
        .unwrap()
}

#[test]
fn snapshot_preserves_authored_subframes_and_negative_time() {
    let m = model(fixture());
    assert_eq!(m.index.minimum_time(), -2.);
    assert_eq!(m.index.key("sx").unwrap().2.time, 0.125);
    assert_eq!(m.index.channel("a", "opacity").unwrap().keys.len(), 2);
}
#[test]
fn snapshot_keeps_unknown_key_fields() {
    let m = model(fixture());
    assert_eq!(
        m.index
            .key("a0")
            .unwrap()
            .2
            .stored
            .get("custom")
            .unwrap()
            .get("keep"),
        Some(&Value::Bool(true))
    );
}
#[test]
fn default_disclosure_matches_tag() {
    let m = model(fixture());
    assert!(!m.index.layer("a").unwrap().collapsed);
    assert!(m.index.layer("b").unwrap().collapsed);
}
#[test]
fn virtual_window_has_bounded_overscan_and_identity_lookup() {
    let m = model(fixture());
    let w = m.window(2, 2, 1);
    assert_eq!(w.first, 1);
    assert_eq!(w.rows.len(), 4);
    for (i, r) in m.rows().iter().enumerate() {
        assert_eq!(m.row_position(&r.id), Some(i));
    }
    assert!(m.window(usize::MAX, usize::MAX, 0).rows.is_empty());
}
#[test]
fn sixty_thousand_rows_remain_virtual_and_pack_without_quadratic_scans() {
    let mut value = fixture();
    let base = value.get("layers").unwrap().as_array().unwrap()[2].clone();
    let mut layers = Vec::new();
    for i in 0..60_001 {
        let mut l = base.clone();
        let map = l.as_object_mut().unwrap();
        map.insert("id", Value::from(format!("L{i}")));
        map.insert("from", Value::Number(i as f64 * 2.));
        map.insert("dur", Value::Number(1.));
        layers.push(l);
    }
    value
        .as_object_mut()
        .unwrap()
        .insert("layers", Value::Array(layers));
    let m = model(value);
    assert_eq!(m.window(30_000, 40, 4).rows.len(), 48);
    assert_eq!(m.rows().len(), 60_001);
    let layout = tracks::layout(&m.index, &m.reveal);
    assert_eq!(layout.items.len(), 60_001);
    assert_eq!(layout.video, 1);
}
#[test]
fn selection_publications_never_rebuild_document_rows() {
    let (_a, mut c) = client();
    let n = c.model.rebuilds;
    for _ in 0..10 {
        select(&mut c, &["a"]);
    }
    assert_eq!(c.model.rebuilds, n);
}
#[test]
fn document_session_change_resets_transient_selection_anchor() {
    let (_a, mut c) = client();
    let p = c.model.select_layer("a", mods()).unwrap();
    for call in p {
        block_on(c.call(call)).unwrap();
    }
    block_on(c.kernel.close("P")).unwrap();
    block_on(c.kernel.open("P", OpenSource::Untitled(fixture())))
        .unwrap()
        .unwrap();
    c.sync().unwrap();
    assert!(c.model.selection.is_empty());
}
#[test]
fn plain_layer_click_retains_multiselection_and_clears_keys() {
    let (_a, mut c) = client();
    select(&mut c, &["a", "b"]);
    select_keys(&mut c, &["a0"]);
    let calls = c.model.select_layer("a", mods()).unwrap();
    for call in calls {
        block_on(c.call(call)).unwrap();
    }
    assert_eq!(c.model.selection.layers(), strings(&["a", "b"]));
    assert!(c.model.selection.keys().is_empty());
}
#[test]
fn command_click_toggles_layer_membership() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    let calls = c
        .model
        .select_layer(
            "a",
            Modifiers {
                meta: true,
                ..mods()
            },
        )
        .unwrap();
    for call in calls {
        block_on(c.call(call)).unwrap();
    }
    assert!(c.model.selection.layers().is_empty());
}
#[test]
fn shift_range_extends_then_shrinks_from_original_anchor() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    for id in ["c", "b"] {
        let calls = c
            .model
            .select_layer(
                id,
                Modifiers {
                    shift: true,
                    ..mods()
                },
            )
            .unwrap();
        for call in calls {
            block_on(c.call(call)).unwrap();
        }
    }
    assert_eq!(c.model.selection.layers(), strings(&["a", "b"]));
}
#[test]
fn command_shift_range_unions_previous_selection() {
    let (_a, mut c) = client();
    select(&mut c, &["a", "c"]);
    let calls = c
        .model
        .select_layer(
            "b",
            Modifiers {
                shift: true,
                control: true,
                ..mods()
            },
        )
        .unwrap();
    for call in calls {
        block_on(c.call(call)).unwrap();
    }
    assert!(c.model.selection.contains("c"));
    assert_eq!(c.model.selection.layers().len(), 3);
}
#[test]
fn additive_key_click_toggles_only_at_pointer_up() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    select_keys(&mut c, &["a0"]);
    let ids = strings(&["a0"]);
    for dragging in [true, false] {
        let calls = c
            .model
            .select_keys("a", "opacity", &ids, true, dragging)
            .unwrap();
        for call in calls {
            block_on(c.call(call)).unwrap();
        }
        if dragging {
            assert_eq!(c.model.selection.keys(), ids);
        }
    }
    assert!(c.model.selection.keys().is_empty());
}
#[test]
fn strip_click_preserves_graph_focus() {
    let (_a, mut c) = client();
    let calls = c
        .model
        .select_keys("a", "opacity", &strings(&["a0"]), false, false)
        .unwrap();
    for call in calls {
        block_on(c.call(call)).unwrap();
    }
    let calls = c.model.select_layer("b", mods()).unwrap();
    for call in calls {
        block_on(c.call(call)).unwrap();
    }
    assert_eq!(c.model.graph_focus, Some(("a".into(), "opacity".into())));
}
#[test]
fn marquee_does_not_seek_and_preserves_additive_keys() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    select_keys(&mut c, &["a0"]);
    let calls = c.model.marquee(strings(&["b"]), strings(&["b0"]), true);
    run(
        &mut c,
        Plan {
            calls,
            ..Default::default()
        },
    );
    assert_eq!(c.model.selection.keys(), strings(&["a0", "b0"]));
}
#[test]
fn group_disclosure_does_not_open_group_properties() {
    let mut v = fixture();
    let a = layer_mut(&mut v, 0);
    a.insert("type", Value::from("group"));
    a.insert("collapsed", Value::Bool(true));
    layer_mut(&mut v, 1).insert("group", Value::from("a"));
    let mut m = model(v);
    assert!(m.row_position(&RowId::Layer("b".into())).is_none());
    m.toggle_disclosure("a").unwrap();
    assert!(m.row_position(&RowId::Layer("b".into())).is_some());
    assert!(
        !m.rows()
            .iter()
            .any(|r| matches!(&r.id,RowId::Property { layer_id,.. } if layer_id=="a"))
    );
}
#[test]
fn cyclic_group_input_is_bounded_and_retains_every_id() {
    let mut v = fixture();
    layer_mut(&mut v, 0).insert("group", Value::from("b"));
    layer_mut(&mut v, 1).insert("group", Value::from("a"));
    let m = model(v);
    assert_eq!(
        m.rows()
            .iter()
            .filter(|r| matches!(r.id, RowId::Layer(_)))
            .count(),
        3
    );
}
#[test]
fn inherited_group_lock_is_indexed() {
    let mut v = fixture();
    layer_mut(&mut v, 0).insert("lock", Value::Bool(true));
    layer_mut(&mut v, 1).insert("group", Value::from("a"));
    let m = model(v);
    assert!(m.index.layer("b").unwrap().effectively_locked);
    assert!(
        Drag::begin(
            &m,
            DragKind::Layers {
                ids: strings(&["b"])
            }
        )
        .is_err()
    );
}
#[test]
fn adjacent_navigation_uses_local_offsets_selected_scope_and_no_wrap() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    assert_eq!(c.model.adjacent_key(1, 1.2, None), Some(2.));
    assert_eq!(c.model.adjacent_key(-1, 1., None), None);
    assert_eq!(
        c.model.adjacent_key(1, 2., Some(("a", "opacity"))),
        Some(3.)
    );
}
#[test]
fn adjacent_navigation_filters_channel() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    block_on(c.call(SelectionCall::Channel(Some("opacity".into())).into())).unwrap();
    assert_eq!(c.model.adjacent_key(1, 1.1, None), Some(3.));
}
#[test]
fn frame_step_and_work_navigation_use_explicit_commands() {
    let (_a, mut c) = client();
    assert_eq!(
        command(&mut c, "nextFrame", 1.),
        vec![HostEffect::Seek(1. + 1. / 30.)]
    );
    assert_eq!(
        command(&mut c, "gotoWorkOut", 0.),
        vec![HostEffect::Seek(9.)]
    );
    assert_eq!(command(&mut c, "gotoEnd", 0.), vec![HostEffect::Seek(10.)]);
}
#[test]
fn layer_navigation_uses_authored_in_and_exclusive_out() {
    let (_a, mut c) = client();
    select(&mut c, &["a", "b"]);
    assert_eq!(
        command(&mut c, "gotoLayerIn", 0.),
        vec![HostEffect::Seek(1.)]
    );
    assert_eq!(
        command(&mut c, "gotoLayerOut", 0.),
        vec![HostEffect::Seek(7.)]
    );
}
#[test]
fn visible_event_navigation_includes_strip_edges_and_real_keys() {
    let (_a, mut c) = client();
    assert_eq!(
        command(&mut c, "nextVisibleEvent", 3.1),
        vec![HostEffect::Seek(5.)]
    );
    assert_eq!(
        command(&mut c, "prevVisibleEvent", 1.1),
        vec![HostEffect::Seek(1.)]
    );
}
#[test]
fn keymap_keeps_control_navigation_distinct_from_command_frames() {
    for (m, id) in [
        (
            Modifiers {
                meta: true,
                ..mods()
            },
            "prevFrame",
        ),
        (
            Modifiers {
                control: true,
                ..mods()
            },
            "timeline.adjacentKeyframe:prev",
        ),
    ] {
        let key = resolve_key(&KeyInput {
            key: "left",
            modifiers: m,
            repeat: false,
            in_field: false,
        })
        .unwrap();
        assert_eq!(key.command, id);
    }
}
#[test]
fn keymap_repeat_and_field_admission_match_tag() {
    assert!(
        resolve_key(&KeyInput {
            key: "space",
            modifiers: mods(),
            repeat: true,
            in_field: false
        })
        .is_none()
    );
    assert!(
        resolve_key(&KeyInput {
            key: "pagedown",
            modifiers: mods(),
            repeat: true,
            in_field: false
        })
        .is_some()
    );
    assert!(
        resolve_key(&KeyInput {
            key: "p",
            modifiers: mods(),
            repeat: false,
            in_field: true
        })
        .is_none()
    );
    assert!(
        resolve_key(&KeyInput {
            key: "p",
            modifiers: Modifiers {
                alt: true,
                ..mods()
            },
            repeat: false,
            in_field: false
        })
        .is_none()
    );
}
#[test]
fn every_timeline_shortcut_resolves_to_registered_behaviour() {
    let mut m = model(fixture());
    for key in [
        "space", "home", "end", "pageup", "pagedown", "j", "k", "i", "o", "[", "]", "b", "n", "f9",
        "p", "s", "r", "t", "a", "u", "m", "f", "e", "l", "escape", "delete",
    ] {
        let s = resolve_key(&KeyInput {
            key,
            modifiers: mods(),
            repeat: false,
            in_field: false,
        })
        .unwrap();
        assert!(
            m.command(
                s.command,
                0.,
                s.frames,
                s.shift_reveal,
                0,
                &PropertyFacts::default()
            )
            .is_ok(),
            "{key}"
        );
    }
}
#[test]
fn unknown_agent_commands_and_ids_are_errors() {
    let mut m = model(fixture());
    assert!(
        m.command("missing", 0., None, false, 0, &PropertyFacts::default())
            .is_err()
    );
    assert!(m.select_layer("gone", mods()).is_err());
    assert!(m.add_marker(f64::NAN).is_err());
}
#[test]
fn ruler_scrub_keeps_exact_subframe_time_and_is_lazy() {
    let m = model(fixture());
    let mut scrub = Scrub::begin(m.index, 0.).unwrap();
    assert_eq!(
        scrub.update(1.0123, false, 0.1).unwrap(),
        HostEffect::Seek(1.0123)
    );
    assert!(!scrub.targets_loaded());
    assert_eq!(scrub.update(-4., false, 0.1).unwrap(), HostEffect::Seek(0.));
    assert_eq!(
        scrub.update(30., false, 0.1).unwrap(),
        HostEffect::Seek(10.)
    );
}
#[test]
fn scrub_shift_changes_mid_gesture_do_not_catch_already_passed_targets() {
    let m = model(fixture());
    let mut scrub = Scrub::begin(m.index, 0.).unwrap();
    scrub.update(4.02, false, 0.1).unwrap();
    assert_eq!(scrub.modifiers(true, 0.1).unwrap(), HostEffect::Seek(4.02));
    assert!(scrub.targets_loaded());
    assert_eq!(scrub.update(5.01, true, 0.1).unwrap(), HostEffect::Seek(5.));
    assert_eq!(scrub.modifiers(false, 0.1).unwrap(), HostEffect::Seek(5.01));
}
#[test]
fn marker_add_snaps_to_frame_and_refuses_duplicate_frame() {
    let (_a, mut c) = client();
    let p = c.model.add_marker(2.012).unwrap();
    run(&mut c, p);
    assert_eq!(c.model.index.markers.len(), 2);
    let p = c.model.add_marker(2.013).unwrap();
    assert!(p.calls.is_empty());
    undo(&mut c);
    assert_eq!(c.model.index.markers.len(), 1);
}
#[test]
fn marker_menu_resolves_id_after_reordering() {
    let (_a, mut c) = client();
    let ctx = Context::Marker {
        id: "marker".into(),
    };
    let p = c.model.add_marker(1.).unwrap();
    run(&mut c, p);
    let p = c.model.menu_action(&ctx, "marker.remove", 0.).unwrap();
    run(&mut c, p);
    assert!(
        c.model
            .index
            .markers
            .iter()
            .all(|m| m.id.as_deref() != Some("marker"))
    );
}
#[test]
fn work_area_keeps_at_least_one_frame() {
    let (_a, mut c) = client();
    command(&mut c, "workIn", 9.9);
    assert!(c.model.index.work[1] - c.model.index.work[0] >= 1. / 30. - 1e-9);
    undo(&mut c);
    assert_eq!(c.model.index.work, [1., 9.]);
}
#[test]
fn work_out_drag_can_extend_composition_and_cancel_restores_it() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::WorkOut)).unwrap();
    block_on(c.drag(sample(5.))).unwrap();
    assert_eq!(c.model.index.duration, 14.);
    block_on(c.finish_drag(false)).unwrap();
    assert_eq!(c.model.index.duration, 10.);
}
#[test]
fn work_move_clamps_as_rigid_range() {
    let m = model(fixture());
    let mut drag = Drag::begin(&m, DragKind::WorkMove).unwrap();
    let commands = drag.commands(sample(9.)).unwrap();
    let (_a, mut c) = client();
    edit(&mut c, commands);
    assert_eq!(c.model.index.work, [2., 10.]);
}
#[test]
fn strip_drag_publishes_live_and_commits_one_undo_step() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::Layers {
        ids: strings(&["a", "b"]),
    }))
    .unwrap();
    for delta in [0.5, 1., 2.] {
        block_on(c.drag(sample(delta))).unwrap();
    }
    assert_eq!(c.model.index.layer("a").unwrap().from, 3.);
    block_on(c.finish_drag(true)).unwrap();
    assert_eq!(c.kernel.published("P").unwrap().history.entries, 1);
    undo(&mut c);
    assert_eq!(c.model.index.layer("a").unwrap().from, 1.);
}
#[test]
fn escape_restores_strip_start_without_undo_entry() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::Layers {
        ids: strings(&["a"]),
    }))
    .unwrap();
    block_on(c.drag(sample(2.))).unwrap();
    block_on(c.finish_drag(false)).unwrap();
    assert_eq!(c.model.index.layer("a").unwrap().from, 1.);
    assert_eq!(c.kernel.published("P").unwrap().history.entries, 0);
}
#[test]
fn group_drag_expands_once_and_preserves_member_spacing() {
    let mut v = fixture();
    layer_mut(&mut v, 0).insert("type", Value::from("group"));
    layer_mut(&mut v, 1).insert("group", Value::from("a"));
    let (_a, mut c) = client_with(v);
    block_on(c.begin_drag(DragKind::Layers {
        ids: strings(&["a"]),
    }))
    .unwrap();
    block_on(c.drag(sample(2.))).unwrap();
    assert_eq!(c.model.index.layer("b").unwrap().from, 7.);
    assert_eq!(c.model.index.layer("a").unwrap().from, 3.);
    block_on(c.finish_drag(true)).unwrap();
}
#[test]
fn trim_in_and_out_respect_minimum_frame() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::TrimOut {
        ids: strings(&["a"]),
    }))
    .unwrap();
    block_on(c.drag(sample(-9.))).unwrap();
    assert!((c.model.index.layer("a").unwrap().duration - 1. / 30.).abs() < 1e-9);
    block_on(c.finish_drag(false)).unwrap();
    block_on(c.begin_drag(DragKind::TrimIn {
        ids: strings(&["a"]),
    }))
    .unwrap();
    block_on(c.drag(sample(9.))).unwrap();
    assert!((c.model.index.layer("a").unwrap().duration - 1. / 30.).abs() < 1e-9);
    block_on(c.finish_drag(true)).unwrap();
}
#[test]
fn quick_offset_layers_keeps_first_anchor_and_subframes() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::QuickOffsetLayers {
        ids: strings(&["a", "b", "c"]),
    }))
    .unwrap();
    block_on(c.drag(sample(0.125))).unwrap();
    assert_eq!(c.model.index.layer("a").unwrap().from, 1.);
    assert_eq!(c.model.index.layer("b").unwrap().from, 5.0625);
    assert_eq!(c.model.index.layer("c").unwrap().from, -1.875);
    block_on(c.finish_drag(true)).unwrap();
}
#[test]
fn key_drag_collision_is_restored_when_pointer_returns() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::Keys {
        ids: strings(&["a0"]),
    }))
    .unwrap();
    block_on(c.drag(sample(2.))).unwrap();
    assert!(c.model.index.key("a2").is_none());
    block_on(c.drag(sample(0.))).unwrap();
    assert_eq!(key_time(&c, "a2"), 2.);
    block_on(c.finish_drag(true)).unwrap();
    assert_eq!(c.kernel.published("P").unwrap().history.entries, 0);
}
#[test]
fn graph_drag_stops_before_unselected_neighbour() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::GraphKeys {
        ids: strings(&["a0"]),
    }))
    .unwrap();
    block_on(c.drag(DragSample {
        value_delta: 5.,
        ..sample(9.)
    }))
    .unwrap();
    assert_eq!(key_time(&c, "a2"), 2.);
    assert!(key_time(&c, "a0") <= 2. - 1. / 30. + 1e-9);
    block_on(c.finish_drag(false)).unwrap();
}
#[test]
fn graph_value_only_drag_keeps_authored_subframe_time() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::GraphKeys {
        ids: strings(&["sx"]),
    }))
    .unwrap();
    block_on(c.drag(DragSample {
        value_delta: 5.,
        ..sample(0.)
    }))
    .unwrap();
    assert_eq!(key_time(&c, "sx"), 0.125);
    block_on(c.finish_drag(true)).unwrap();
}
#[test]
fn option_stretch_uses_opposite_endpoint_anchor() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::StretchKeys {
        ids: strings(&["a0", "a2"]),
        anchor_time: 1.,
        grabbed_time: 3.,
    }))
    .unwrap();
    block_on(c.drag(sample(2.))).unwrap();
    assert_eq!(key_time(&c, "a0"), 0.);
    assert_eq!(key_time(&c, "a2"), 4.);
    block_on(c.finish_drag(true)).unwrap();
    undo(&mut c);
    assert_eq!(key_time(&c, "a2"), 2.);
}
#[test]
fn graph_transform_scales_numeric_values() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::TransformKeys {
        ids: strings(&["a0", "a2"]),
        anchor_time: 1.,
        anchor_value: 10.,
    }))
    .unwrap();
    block_on(c.drag(DragSample {
        value_scale: 2.,
        ..sample(0.)
    }))
    .unwrap();
    assert_eq!(c.model.index.key("a2").unwrap().2.value, Value::Number(90.));
    block_on(c.finish_drag(false)).unwrap();
}
#[test]
fn ease_shortcuts_use_kernel_interpolation_and_undo() {
    let (_a, mut c) = client();
    select_keys(&mut c, &["a0"]);
    command(&mut c, "easyEaseOut", 0.);
    let key = &c.model.index.key("a0").unwrap().2.stored;
    assert!(key.get("outEase").is_some());
    undo(&mut c);
    assert!(
        c.model
            .index
            .key("a0")
            .unwrap()
            .2
            .stored
            .get("outEase")
            .is_none()
    );
}
#[test]
fn handle_drag_is_undoable_and_option_splits_continuity() {
    let (_a, mut c) = client();
    edit(
        &mut c,
        vec![
            KeyframeCommand::SetInterpolation {
                key_ids: strings(&["a0"]),
                side: EaseSide::Both,
                interpolation: Interpolation::Continuous,
            }
            .into(),
        ],
    );
    block_on(c.begin_drag(DragKind::Handle {
        id: "a0".into(),
        side: EaseSide::Out,
    }))
    .unwrap();
    block_on(c.drag(DragSample {
        handle_time: 0.5,
        handle_value: 20.,
        modifiers: Modifiers {
            alt: true,
            ..mods()
        },
        ..sample(0.)
    }))
    .unwrap();
    block_on(c.finish_drag(true)).unwrap();
    assert_eq!(
        c.model.index.key("a0").unwrap().2.stored.get("continuous"),
        Some(&Value::Bool(false))
    );
}
#[test]
fn menu_easing_uses_current_selected_keys_only_if_hit_selected() {
    let (_a, mut c) = client();
    select_keys(&mut c, &["a0", "a2"]);
    assert_eq!(
        c.model.context_keys(&strings(&["a0"])),
        strings(&["a0", "a2"])
    );
    assert_eq!(c.model.context_keys(&strings(&["b0"])), strings(&["b0"]));
    let ctx = Context::Keys {
        ids: strings(&["a0"]),
    };
    let p = c.model.menu_action(&ctx, "ease.linear", 0.).unwrap();
    run(&mut c, p);
}
#[test]
fn hold_context_toggle_applies_to_whole_selected_scope() {
    let (_a, mut c) = client();
    select_keys(&mut c, &["a0", "a2"]);
    let ctx = Context::Keys {
        ids: strings(&["a0"]),
    };
    let p = c.model.menu_action(&ctx, "keys.hold", 0.).unwrap();
    run(&mut c, p);
    for id in ["a0", "a2"] {
        assert_eq!(
            c.model
                .index
                .key(id)
                .unwrap()
                .2
                .stored
                .get("outInterp")
                .and_then(Value::as_str),
            Some("hold")
        );
    }
}
#[test]
fn clipboard_retains_unknown_fields_and_composition_offsets() {
    let m = model(fixture());
    let tracks = m.copy_keys(&strings(&["a0", "b0"]));
    assert_eq!(tracks[0].keys[0].at, 0.);
    assert_eq!(tracks[1].keys[0].at, 4.);
    assert!(tracks[0].keys[0].key.get("custom").is_some());
}
#[test]
fn paste_selects_new_ids_and_is_undoable() {
    let (_a, mut c) = client();
    let copied = c.model.copy_keys(&strings(&["a0", "a2"]));
    let p = c
        .model
        .paste_keys("b", "opacity", 5., &copied[0].keys)
        .unwrap();
    run(&mut c, p);
    assert_eq!(c.model.selection.keys().len(), 2);
    let id = c.model.selection.keys()[1].clone();
    assert_eq!(key_time(&c, &id), 2.);
    undo(&mut c);
    assert!(c.model.index.key(&id).is_none());
}
#[test]
fn name_draft_enter_commits_escape_and_blank_do_not_write() {
    let (_a, mut c) = client();
    for (text, commit) in [("Changed", false), ("   ", true)] {
        let p = Draft::Name {
            layer_id: "a".into(),
            text: text.into(),
        }
        .finish(commit, c.model.index.revision);
        assert!(p.calls.is_empty());
    }
    let p = Draft::Name {
        layer_id: "a".into(),
        text: "New name".into(),
    }
    .finish(true, c.model.index.revision);
    run(&mut c, p);
    assert_eq!(
        c.model.index.layer("a").unwrap().name.to_string_lossy(),
        "New name"
    );
    undo(&mut c);
}
#[test]
fn property_draft_updates_multiple_layers_atomically() {
    let (_a, mut c) = client();
    let p = Draft::Property {
        targets: vec![
            ("a".into(), "opacity".into()),
            ("c".into(), "opacity".into()),
        ],
        value: Value::Number(35.),
        time: 1.,
    }
    .finish(true, c.model.index.revision);
    run(&mut c, p);
    assert_eq!(
        c.model.index.channel("c", "opacity").unwrap().value,
        Value::Number(35.)
    );
    assert_eq!(c.kernel.published("P").unwrap().history.entries, 1);
}
#[test]
fn visibility_solo_lock_switches_go_through_transactions() {
    let (_a, mut c) = client();
    for s in [LayerSwitch::Visible, LayerSwitch::Solo, LayerSwitch::Locked] {
        let p = c.model.switch("a", s).unwrap();
        run(&mut c, p);
    }
    let l = c.model.index.layer("a").unwrap();
    assert!(!l.visible);
    assert!(l.solo);
    assert!(l.locked);
    let p = c.model.switch("a", LayerSwitch::Locked).unwrap();
    run(&mut c, p);
    assert!(!c.model.index.layer("a").unwrap().locked);
}
#[test]
fn stale_view_commands_refuse_without_overwriting_foreign_edit() {
    let (_a, mut c) = client();
    let p = c.model.switch("a", LayerSwitch::Visible).unwrap();
    edit(
        &mut c,
        vec![Command::SetLayerName {
            layer_id: "a".into(),
            name: "Foreign".into(),
        }],
    );
    assert!(block_on(c.dispatch(p)).is_err());
    assert!(c.model.index.layer("a").unwrap().visible);
}
#[test]
fn agent_origin_cannot_unlock_and_edit_locked_layer() {
    let mut v = fixture();
    layer_mut(&mut v, 0).insert("lock", Value::Bool(true));
    let (_a, mut c) = client_with(v);
    c.caller = Caller::realm(301, 1, Origin::Mcp);
    let p = c.model.switch("a", LayerSwitch::Locked).unwrap();
    assert!(block_on(c.dispatch(p)).is_err());
    assert!(c.model.index.layer("a").unwrap().locked);
}
#[test]
fn idempotent_agent_retry_does_not_apply_twice() {
    let (_a, mut c) = client();
    c.caller = Caller::realm(302, 1, Origin::Mcp);
    let cmd = Command::SetLayerName {
        layer_id: "a".into(),
        name: "Agent".into(),
    };
    let first = block_on(c.transact_idempotent(vec![cmd.clone()], 77)).unwrap();
    let second = block_on(c.transact_idempotent(vec![cmd], 77)).unwrap();
    assert_eq!(first.revision, second.revision);
    assert_eq!(c.kernel.published("P").unwrap().history.entries, 1);
}
#[test]
fn cancel_preserves_acknowledged_foreign_layer_name() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::Layers {
        ids: strings(&["a"]),
    }))
    .unwrap();
    block_on(c.drag(sample(2.))).unwrap();
    let foreign = Caller::realm(303, 1, Origin::Agent);
    let call = Call::Transact {
        label: None,
        body: WriteBody::Commands(vec![Command::SetLayerName {
            layer_id: "a".into(),
            name: "Kept".into(),
        }]),
        expected_revision: None,
    };
    let ack = block_on(c.kernel.call("P", Request::new(foreign, call)))
        .unwrap()
        .unwrap();
    assert!(!matches!(ack.reply, Reply::Throw(_)));
    block_on(c.finish_drag(false)).unwrap();
    assert_eq!(
        c.model.index.layer("a").unwrap().name.to_string_lossy(),
        "Kept"
    );
    assert_eq!(c.model.index.layer("a").unwrap().from, 1.);
}
#[test]
fn split_selects_tail_and_keeps_source_animation_time() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    command(&mut c, "split", 2.);
    assert_eq!(c.model.selection.layers().len(), 1);
    let tail = c.model.selection.layers()[0].clone();
    let l = c.model.index.layer(&tail).unwrap();
    assert_eq!(l.from, 2.);
    let times: Vec<_> = l
        .channels
        .iter()
        .find(|ch| ch.path == "opacity")
        .unwrap()
        .keys
        .iter()
        .map(|k| l.from + k.time)
        .collect();
    assert_eq!(times, vec![1., 3.]);
    undo(&mut c);
    assert!(c.model.index.layer(&tail).is_none());
}
#[test]
fn reveal_single_shift_toggle_and_double_tap_are_ui_only() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    let revision = c.model.index.revision;
    let facts = PropertyFacts::default();
    c.model.reveal_property('p', false, 0, &facts).unwrap();
    assert_eq!(
        c.model.reveal.layers["a"].paths,
        Some(strings(&["position.x"]))
    );
    c.model.reveal_property('t', true, 400, &facts).unwrap();
    assert!(
        c.model.reveal.layers["a"]
            .paths
            .as_ref()
            .unwrap()
            .contains(&"opacity".into())
    );
    c.model.reveal_property('t', true, 800, &facts).unwrap();
    assert!(
        !c.model.reveal.layers["a"]
            .paths
            .as_ref()
            .unwrap()
            .contains(&"opacity".into())
    );
    c.model.reveal_property('e', false, 1200, &facts).unwrap();
    c.model.reveal_property('e', false, 1400, &facts).unwrap();
    assert_eq!(c.model.index.revision, revision);
    assert_eq!(c.kernel.published("P").unwrap().history.entries, 0);
}
/// v1–3's `TimelineService.reveal`: opens the layer and adds the keys to
/// what it showed (its animated channels), UI state only.
#[test]
fn service_reveal_adds_keys_to_the_animated_channels() {
    let (_a, mut c) = client();
    let revision = c.model.index.revision;
    c.model.reveal_paths("a", &strings(&["rotation"])).unwrap();
    let shown = &c.model.reveal.layers["a"];
    assert!(!shown.collapsed);
    assert_eq!(
        shown.paths,
        Some(strings(&[
            "opacity",
            "position.x",
            "scale.x",
            "scale.y",
            "rotation"
        ]))
    );
    c.model.reveal_paths("a", &strings(&["rotation"])).unwrap();
    assert_eq!(
        c.model.reveal.layers["a"].paths,
        Some(strings(&[
            "opacity",
            "position.x",
            "scale.x",
            "scale.y",
            "rotation"
        ]))
    );
    assert!(c.model.reveal_paths("missing", &[]).is_err());
    assert_eq!(c.model.index.revision, revision);
}
#[test]
fn reveal_double_u_includes_schema_reported_modified_properties() {
    let mut m = model(fixture());
    let mut facts = PropertyFacts::default();
    facts.modified.insert(("c".into(), "opacity".into()));
    m.reveal_property('u', false, 0, &facts).unwrap();
    m.reveal_property('u', false, 299, &facts).unwrap();
    assert!(
        m.reveal.layers["c"]
            .paths
            .as_ref()
            .unwrap()
            .contains(&"opacity".into())
    );
}
#[test]
fn global_m_is_repeatable_disclosure_not_a_double_tap() {
    let mut m = model(fixture());
    m.reveal_property('m', false, 0, &PropertyFacts::default())
        .unwrap();
    assert!(m.reveal.layers.values().all(|d| !d.collapsed));
    m.reveal_property('m', false, 1, &PropertyFacts::default())
        .unwrap();
    assert!(m.reveal.layers.values().all(|d| d.collapsed));
}
#[test]
fn graph_and_timeline_modes_do_not_touch_document() {
    let (_a, mut c) = client();
    let rev = c.model.index.revision;
    for id in [
        "graph",
        "graph.speed",
        "graph.combined",
        "timeline.mode:tracks",
        "timeline.toggleMode",
    ] {
        command(&mut c, id, 0.);
    }
    assert_eq!(c.model.index.revision, rev);
    assert_eq!(c.model.mode, TimelineMode::Layers);
    assert_eq!(c.model.graph_mode, GraphMode::Speed);
}
#[test]
fn graph_menu_eligibility_tracks_series_count() {
    let m = model(fixture());
    for series in 0..3 {
        let menu = m.menu(&Context::Graph { series });
        assert_eq!(
            menu.iter().find(|i| i.id == "graph.fit").unwrap().enabled,
            series > 0
        );
        assert_eq!(
            menu.iter().find(|i| i.id == "graph.merge").unwrap().enabled,
            series >= 2
        );
    }
}
#[test]
fn track_context_disables_clear_for_static_property() {
    let m = model(fixture());
    let menu = m.menu(&Context::Track {
        layer: "c".into(),
        channel: "opacity".into(),
    });
    assert!(!menu.iter().find(|i| i.id == "track.clear").unwrap().enabled);
}
#[test]
fn clipboard_and_dialogs_are_explicit_host_effects() {
    let mut m = model(fixture());
    let ctx = Context::Keys {
        ids: strings(&["a0"]),
    };
    assert!(matches!(
        m.menu_action(&ctx, "keys.value", 0.)
            .unwrap()
            .effects
            .as_slice(),
        [HostEffect::EditKeyValues(_)]
    ));
    assert!(matches!(
        m.menu_action(&ctx, "keys.copy", 0.)
            .unwrap()
            .effects
            .as_slice(),
        [HostEffect::CopyKeys(_)]
    ));
    assert!(m.menu_action(&ctx, "missing", 0.).is_err());
}
#[test]
fn scale_key_selection_expands_both_axes_without_retiming() {
    let m = model(fixture());
    assert_eq!(
        m.expand_scale_keys(&strings(&["sx"])),
        strings(&["sx", "sy"])
    );
}
#[test]
fn property_diamond_adds_missing_targets_then_removes_all() {
    let (_a, mut c) = client();
    let targets = vec![
        ("a".into(), "opacity".into()),
        ("c".into(), "opacity".into()),
    ];
    let p = c.model.property_diamond(&targets, 1.).unwrap();
    run(&mut c, p);
    assert_eq!(c.model.index.channel("a", "opacity").unwrap().keys.len(), 2);
    assert_eq!(c.model.index.channel("c", "opacity").unwrap().keys.len(), 1);
    let p = c.model.property_diamond(&targets, 1.).unwrap();
    run(&mut c, p);
    assert_eq!(c.model.index.channel("a", "opacity").unwrap().keys.len(), 1);
    assert!(
        c.model
            .index
            .channel("c", "opacity")
            .unwrap()
            .keys
            .is_empty()
    );
}
#[test]
fn stopwatch_is_undoable() {
    let (_a, mut c) = client();
    let p = c.model.stopwatch("c", "opacity", 1.).unwrap();
    run(&mut c, p);
    assert_eq!(c.model.index.channel("c", "opacity").unwrap().keys.len(), 1);
    undo(&mut c);
    assert!(
        c.model
            .index
            .channel("c", "opacity")
            .unwrap()
            .keys
            .is_empty()
    );
}
#[test]
fn numeric_field_drag_observes_fine_fast_clamps_and_precision() {
    let meta = NumericMetadata {
        step: 0.5,
        min: 0.,
        max: 100.,
    };
    assert_eq!(dragged_value(10., 4., meta, true, true).unwrap(), 10.2);
    assert_eq!(dragged_value(10., 4., meta, false, true).unwrap(), 30.);
    assert_eq!(dragged_value(99., 4., meta, false, false).unwrap(), 100.);
    assert!(dragged_value(0., f64::NAN, meta, false, false).is_err());
}
#[test]
fn viewport_pan_zoom_and_negative_time_have_no_visual_constants() {
    let mut v = TimeViewport::new(0., 10.).unwrap();
    v.pan(-5., -2.).unwrap();
    assert_eq!((v.start, v.end), (-2., 8.));
    v.zoom(3., 2., -2.).unwrap();
    assert_eq!((v.start, v.end), (0.5, 5.5));
    assert!(v.zoom(0., 0., 0.).is_err());
}
#[test]
fn composition_tab_close_predicate_and_keyboard_open_are_host_semantics() {
    let tabs = composition_tabs(&strings(&["C", "Other"]), "C");
    assert!(tabs[0].active);
    assert!(tabs.iter().all(|t| t.closable));
    assert!(!composition_tabs(&strings(&["C"]), "C")[0].closable);
}
#[test]
fn cue_modifiers_toggle_without_starting_drag() {
    let mut s = CueSelection::default();
    assert!(s.click("caption", "one", false));
    assert!(!s.click("caption", "two", true));
    assert_eq!(s.ids, strings(&["one", "two"]));
    assert!(!s.click("caption", "one", true));
    assert_eq!(s.ids, strings(&["two"]));
    s.clear();
    assert_eq!(s, CueSelection::default());
}
#[test]
fn caption_moves_reach_the_kernel_and_undo_retains_cues() {
    let mut v = fixture();
    layer_mut(&mut v, 2).insert("type", Value::from("captions"));
    layer_mut(&mut v, 2).insert("d", pm_model::json::parse(r#"{"cues":[{"id":"cue","start":1,"end":2,"text":"Synthetic"}],"opaque":{"keep":true}}"#).unwrap());
    let (_a, mut c) = client_with(v);
    let p = c
        .model
        .caption_edit(
            "c",
            pm_kernel::commands::captions::CaptionEdit::Move {
                ids: strings(&["cue"]),
                by: 1.,
            },
        )
        .unwrap();
    let published = c.kernel.published(&c.project).unwrap();
    let before = published.snapshot.document.to_value().unwrap();
    let before_revision = published.revision();
    block_on(c.dispatch(p)).unwrap();
    assert!(c.kernel.published(&c.project).unwrap().revision() > before_revision);
    block_on(c.call(pm_kernel::HistoryCall::Undo.into())).unwrap();
    let after = c
        .kernel
        .published(&c.project)
        .unwrap()
        .snapshot
        .document
        .to_value()
        .unwrap();
    // History preserves authored content; revision is a separate monotonic stamp.
    let layer = |document: pm_model::Value| {
        let document = pm_model::canonical::realize_project(document);
        document
            .get("layers")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l.get("id").and_then(Value::as_str) == Some("c"))
            .unwrap()
            .get("d")
            .unwrap()
            .clone()
    };
    assert_eq!(layer(before), layer(after));
}
#[test]
fn import_drop_is_id_addressed_and_option_requests_folder() {
    let m = model(fixture());
    assert_eq!(
        m.import_drop(1.012, Some("b".into()), true).unwrap(),
        HostEffect::Import {
            time: 1.,
            before_layer: Some("b".into()),
            as_folder: true
        }
    );
    assert!(m.import_drop(1., Some("gone".into()), false).is_err());
}
#[test]
fn text_stagger_body_and_stretch_use_existing_timing_channels() {
    let band = TextStagger {
        delay: 0.,
        duration: 1.,
        stagger: 0.2,
        cascade: 1.,
    };
    let body = band.commands("a", "TA", false, 1.01, 30., 0.).unwrap();
    assert!(
        matches!(&body[0],Command::Property(PropertyCommand::Set(s)) if s.channel=="ta.TA.delay" && s.value==Value::Number(1.))
    );
    let end = band.commands("a", "TA", true, 2., 30., 0.).unwrap();
    assert_eq!(end.len(), 2);
    assert!(
        matches!(&end[0],Command::Property(PropertyCommand::Set(s)) if s.value==Value::Number(2.))
    );
}
#[test]
fn quick_offset_keys_preserves_subframe_total() {
    let (_a, mut c) = client();
    let p = c
        .model
        .quick_offset_keys(&[strings(&["a0"]), strings(&["b0"])], 0.125, 0.)
        .unwrap();
    run(&mut c, p);
    assert_eq!(key_time(&c, "a0"), 0.);
    assert_eq!(key_time(&c, "b0"), 0.125);
}

fn oracle() -> Value {
    parse(include_str!("fixtures/tag-oracle.json")).unwrap()
}
fn n(value: &Value, key: &str) -> f64 {
    value.get(key).unwrap().as_f64().unwrap()
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).unwrap().as_str().unwrap()
}
fn edge(value: Option<&Value>) -> Option<Edge> {
    match value.and_then(Value::as_str) {
        Some("in") => Some(Edge::In),
        Some("out") => Some(Edge::Out),
        _ => None,
    }
}
#[test]
fn track_layout_matches_forty_tag_generated_group_audio_caption_and_overlap_cases() {
    for (case_no, case) in oracle()
        .get("tracks")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let mut value = fixture();
        value
            .as_object_mut()
            .unwrap()
            .insert("layers", case.get("layers").unwrap().clone());
        value
            .as_object_mut()
            .unwrap()
            .insert("dur", Value::Number(20.));
        let mut m = model(value);
        for id in case.get("collapsed").unwrap().as_array().unwrap() {
            m.reveal
                .layers
                .entry(id.as_str().unwrap().into())
                .or_default()
                .group_collapsed = true;
        }
        // Explicitly open other groups: default legacy disclosure is closed.
        for l in &m.index.layers {
            m.reveal.layers.entry(l.id.clone()).or_default();
        }
        let out = tracks::layout(&m.index, &m.reveal);
        let expected = case.get("expected").unwrap();
        assert_eq!(
            (out.video, out.audio, out.captions),
            (
                n(expected, "video") as usize,
                n(expected, "audio") as usize,
                n(expected, "captions") as usize
            ),
            "case {case_no}"
        );
        let items = expected.get("items").unwrap().as_array().unwrap();
        assert_eq!(out.items.len(), items.len());
        for (actual, want) in out.items.iter().zip(items) {
            assert_eq!(actual.id, text(want, "id"), "case {case_no}");
            assert_eq!(actual.lane, n(want, "lane") as usize);
            assert!((actual.from - n(want, "from")).abs() < 1e-9);
            assert!((actual.duration - n(want, "dur")).abs() < 1e-9);
            assert_eq!(actual.depth, n(want, "depth") as usize);
            assert_eq!(
                actual.area,
                match text(want, "area") {
                    "video" => tracks::Area::Video,
                    "audio" => tracks::Area::Audio,
                    _ => tracks::Area::Captions,
                }
            );
        }
    }
}
#[test]
fn crossing_latch_matches_960_tag_samples_including_reversals_and_modifier_changes() {
    for case in oracle().get("snapping").unwrap().as_array().unwrap() {
        let anchors: Vec<_> = case
            .get("anchors")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|a| Anchor {
                time: n(a, "time"),
                edge: edge(a.get("edge")),
            })
            .collect();
        let targets: Vec<_> = case
            .get("targets")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|a| Target {
                time: n(a, "time"),
                edge: edge(a.get("edge")),
                weight: n(a, "weight"),
            })
            .collect();
        let mut snap = DirectionalSnap::default();
        for step in case.get("steps").unwrap().as_array().unwrap() {
            let delta = snap.resolve(
                &anchors,
                n(step, "raw"),
                &targets,
                0.15,
                step.get("enabled").unwrap().as_bool().unwrap(),
            );
            assert!((delta - n(step, "delta")).abs() < 1e-9);
            match step.get("lock").unwrap() {
                Value::Null => assert!(snap.lock.is_none()),
                want => {
                    let actual = snap.lock.unwrap();
                    assert_eq!(actual.anchor, n(want, "anchor") as usize);
                    assert_eq!(actual.direction, n(want, "direction") as i8);
                    assert!((actual.target - n(want, "target")).abs() < 1e-9);
                }
            }
        }
    }
}
#[test]
fn snap_does_not_attract_nearby_edges_before_crossing() {
    let mut s = DirectionalSnap::default();
    let a = [Anchor {
        time: 0.,
        edge: None,
    }];
    assert_eq!(s.resolve(&a, 0.95, &[Target::at(1.)], 0.1, true), 0.95);
    assert_eq!(s.resolve(&a, 1.01, &[Target::at(1.)], 0.1, true), 1.);
    assert_eq!(s.resolve(&a, 0.99, &[Target::at(1.)], 0.1, true), 0.99);
}
#[test]
fn snap_release_observes_removed_targets_and_incompatible_edges() {
    let mut s = DirectionalSnap::default();
    let a = [Anchor {
        time: 0.,
        edge: Some(Edge::In),
    }];
    let target = Target {
        time: 1.,
        edge: Some(Edge::Out),
        weight: 1.,
    };
    assert_eq!(s.resolve(&a, 1.01, &[target], 0.1, true), 1.01);
    s.reset(0.);
    assert_eq!(s.resolve(&a, 1.01, &[Target::at(1.)], 0.1, true), 1.);
    assert_eq!(s.resolve(&a, 1.02, &[], 0.1, true), 1.02);
}
#[test]
fn snap_weights_only_break_equal_distance_ties() {
    let mut s = DirectionalSnap::default();
    let a = [Anchor {
        time: 0.,
        edge: None,
    }];
    assert_eq!(
        s.resolve(
            &a,
            1.05,
            &[
                Target {
                    time: 1.,
                    edge: None,
                    weight: 100.
                },
                Target::at(1.04)
            ],
            0.1,
            true
        ),
        1.04
    );
}
#[test]
fn invalid_snap_samples_hold_last_raw_time() {
    let mut s = DirectionalSnap::new(0.5);
    assert_eq!(s.resolve(&[], f64::NAN, &[], f64::NAN, false), 0.5);
}
#[test]
fn track_move_preview_matches_forty_tag_cases_including_group_and_area_slots() {
    for (case_no, case) in oracle()
        .get("tracks")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let mut value = fixture();
        value
            .as_object_mut()
            .unwrap()
            .insert("layers", case.get("layers").unwrap().clone());
        value
            .as_object_mut()
            .unwrap()
            .insert("dur", Value::Number(20.));
        let mut m = model(value);
        for l in &m.index.layers {
            m.reveal.layers.entry(l.id.clone()).or_default();
        }
        for id in case.get("collapsed").unwrap().as_array().unwrap() {
            m.reveal
                .layers
                .get_mut(id.as_str().unwrap())
                .unwrap()
                .group_collapsed = true;
        }
        let mv = case.get("move").unwrap();
        let lanes = mv.get("lanes").unwrap();
        let ids: Vec<_> = mv
            .get("ids")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().into())
            .collect();
        let actual = tracks::plan_move(
            &m.index,
            &m.reveal,
            &ids,
            n(mv, "dt"),
            tracks::LaneDelta {
                video: n(lanes, "video") as isize,
                audio: n(lanes, "audio") as isize,
                captions: n(lanes, "captions") as isize,
            },
        )
        .unwrap();
        let expected = case.get("moved").unwrap().as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        for (actual, want) in actual.iter().zip(expected) {
            assert_eq!(actual.id, text(want, "id"), "case {case_no}");
            assert_eq!(
                actual.track,
                n(want, "track") as usize,
                "case {case_no} id {}",
                actual.id
            );
            assert_eq!(actual.from, n(want, "from"));
        }
    }
}
#[test]
fn keyframe_proximity_snap_is_distinct_from_strip_crossing() {
    let m = model(fixture());
    let mut drag = Drag::begin(
        &m,
        DragKind::Keys {
            ids: strings(&["a0"]),
        },
    )
    .unwrap();
    drag.commands(DragSample {
        modifiers: Modifiers {
            shift: true,
            ..mods()
        },
        acquire_tolerance: 0.1,
        release_tolerance: 0.15,
        ..sample(1.95)
    })
    .unwrap();
    assert_eq!(drag.key_snap.unwrap().target, 3.);
}
#[test]
fn track_magnet_snaps_by_default_and_shift_temporarily_inverts_it() {
    let mut m = model(fixture());
    m.mode = TimelineMode::Tracks;
    let mut drag = Drag::begin(
        &m,
        DragKind::Layers {
            ids: strings(&["a"]),
        },
    )
    .unwrap();
    drag.commands(DragSample {
        release_tolerance: 0.1,
        ..sample(0.01)
    })
    .unwrap();
    drag.commands(DragSample {
        release_tolerance: 0.1,
        ..sample(2.01)
    })
    .unwrap();
    assert!(drag.snap.lock.is_some());
    drag.commands(DragSample {
        modifiers: Modifiers {
            shift: true,
            ..mods()
        },
        release_tolerance: 0.1,
        ..sample(2.02)
    })
    .unwrap();
    assert!(drag.snap.lock.is_none());
}
#[test]
fn playhead_targets_use_visible_frames_not_exclusive_out_edges() {
    let m = model(fixture());
    let targets = playhead_targets(&m.index);
    assert!(targets.contains(&(7. - 1. / 30.)));
    assert!(!targets.contains(&7.));
    assert_eq!(first_frame(0.01, 30.), 1. / 30.);
    assert!((last_frame(0., 1., 30.) - 29. / 30.).abs() < 1e-12);
}
#[test]
fn animated_property_drag_starts_at_evaluated_value() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::Property {
        targets: vec![("a".into(), "opacity".into())],
        time: 2.,
    }))
    .unwrap();
    block_on(c.drag(DragSample {
        value_delta: 5.,
        ..sample(0.)
    }))
    .unwrap();
    let p = c.kernel.published("P").unwrap();
    let mut mirror = pm_kernel::QueryMirror::new(&p);
    let value = mirror
        .queries()
        .evaluate(Some("C"), "a", "opacity", 2.)
        .unwrap()
        .as_f64()
        .unwrap();
    assert!((value - 35.).abs() < 1e-7);
    block_on(c.finish_drag(false)).unwrap();
}
#[test]
fn ui_session_patch_preserves_opaque_and_legacy_fields() {
    let ui = pm_kernel::state::UiState { timeline:parse(r#"{"pps":75,"scrollT":-2,"extension":{"keep":true},"nativeModel":{"unknown":1,"disclosure":{"a":{"future":true}}}}"#).unwrap(), ..Default::default() };
    let mut m = model(fixture());
    m.toggle_disclosure("a").unwrap();
    m.mode = TimelineMode::Tracks;
    let patched = m.patch_ui(&ui);
    assert_eq!(patched.timeline.get("pps"), Some(&Value::Number(75.)));
    assert_eq!(
        patched.timeline.get("extension"),
        ui.timeline.get("extension")
    );
    assert_eq!(
        patched
            .timeline
            .get("nativeModel")
            .unwrap()
            .get("disclosure")
            .unwrap()
            .get("a")
            .unwrap()
            .get("future"),
        Some(&Value::Bool(true))
    );
    let mut restored = model(fixture());
    restored.restore_ui(&patched);
    assert_eq!(restored.mode, TimelineMode::Tracks);
    assert_eq!(restored.reveal.layers["a"], m.reveal.layers["a"]);
}
#[test]
fn ui_publication_never_creates_document_undo() {
    let (_a, mut c) = client();
    let rev = c.model.index.revision;
    c.model.mode = TimelineMode::Tracks;
    block_on(c.publish_view()).unwrap();
    assert_eq!(c.model.index.revision, rev);
    assert_eq!(c.kernel.published("P").unwrap().history.entries, 0);
    assert_eq!(
        c.kernel.state().ui["P"]
            .timeline
            .get("mode")
            .and_then(Value::as_str),
        Some("tracks")
    );
}
#[test]
fn track_shift_selection_toggles_instead_of_selecting_range() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    let (calls, drag) = c
        .model
        .select_track_layer(
            "c",
            Modifiers {
                shift: true,
                ..mods()
            },
        )
        .unwrap();
    run(
        &mut c,
        Plan {
            calls,
            ..Default::default()
        },
    );
    assert!(drag);
    assert_eq!(c.model.selection.layers(), strings(&["a", "c"]));
    let (_, drag) = c
        .model
        .select_track_layer(
            "a",
            Modifiers {
                shift: true,
                ..mods()
            },
        )
        .unwrap();
    assert!(!drag);
}
#[test]
fn track_header_select_lock_visibility_and_disclosure_target_its_layers() {
    let (_a, mut c) = client();
    let layout = tracks::layout(&c.model.index, &c.model.reveal);
    let lane = layout.item("b").unwrap().lane;
    let expected: Vec<_> = layout
        .items
        .iter()
        .filter(|i| i.area == tracks::Area::Video && i.lane == lane)
        .map(|i| i.id.clone())
        .collect();
    let p = c
        .model
        .track_header(tracks::Area::Video, lane, TrackHeaderAction::Select);
    run(&mut c, p);
    assert_eq!(c.model.selection.layers(), expected);
    let p = c
        .model
        .track_header(tracks::Area::Video, lane, TrackHeaderAction::Visible);
    run(&mut c, p);
    for id in &expected {
        assert!(!c.model.index.layer(id).unwrap().visible);
    }
    let p = c
        .model
        .track_header(tracks::Area::Video, lane, TrackHeaderAction::Disclosure);
    assert!(p.calls.is_empty());
    let p = c
        .model
        .track_header(tracks::Area::Video, lane, TrackHeaderAction::Lock);
    run(&mut c, p);
    for id in &expected {
        assert!(c.model.index.layer(id).unwrap().locked);
    }
}
#[test]
fn reorder_moves_a_selected_block_before_and_after_by_target_id() {
    let (_a, mut c) = client();
    let p = c
        .model
        .reorder(&strings(&["a", "b"]), "c", true, 0.)
        .unwrap();
    run(&mut c, p);
    assert_eq!(
        c.model
            .index
            .layers
            .iter()
            .map(|l| l.id.as_str())
            .collect::<Vec<_>>(),
        vec!["c", "a", "b"]
    );
    undo(&mut c);
    let p = c
        .model
        .reorder(&strings(&["c", "b"]), "a", false, 0.)
        .unwrap();
    run(&mut c, p);
    assert_eq!(
        c.model
            .index
            .layers
            .iter()
            .map(|l| l.id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "c", "a"]
    );
}
#[test]
fn drop_inside_group_preserves_order_and_is_one_undo_entry() {
    let mut v = fixture();
    fill_transforms(&mut v);
    layer_mut(&mut v, 0).insert("type", Value::from("group"));
    let (_a, mut c) = client_with(v);
    let p = c
        .model
        .drop_layers(
            &strings(&["b", "c"]),
            "a",
            DropMode::Inside,
            0.,
            Origin::Interface,
        )
        .unwrap();
    run(&mut c, p);
    assert_eq!(
        c.model.index.layer("b").unwrap().group.as_deref(),
        Some("a")
    );
    assert_eq!(
        c.model.index.layer("c").unwrap().group.as_deref(),
        Some("a")
    );
    assert_eq!(c.kernel.published("P").unwrap().history.entries, 1);
    undo(&mut c);
    assert!(c.model.index.layer("b").unwrap().group.is_none());
}
#[test]
fn reorder_rejects_self_group_cycles_and_locked_destination() {
    let mut v = fixture();
    layer_mut(&mut v, 0).insert("type", Value::from("group"));
    layer_mut(&mut v, 1).insert("group", Value::from("a"));
    layer_mut(&mut v, 2).insert("lock", Value::Bool(true));
    let m = model(v);
    assert!(m.reorder(&strings(&["a"]), "b", false, 0.).is_err());
    assert!(m.reorder(&strings(&["a"]), "c", false, 0.).is_err());
}
#[test]
fn track_move_commits_preferences_stack_and_timing_in_one_undo_step() {
    let (_a, mut c) = client();
    let p = c
        .model
        .move_tracks(
            &strings(&["b"]),
            0.125,
            tracks::LaneDelta {
                video: 1,
                ..Default::default()
            },
            Origin::Interface,
        )
        .unwrap();
    run(&mut c, p);
    assert_eq!(c.model.index.layer("b").unwrap().from, 5.125);
    assert!(c.model.index.layer("b").unwrap().preferred_track > 0);
    assert_eq!(c.kernel.published("P").unwrap().history.entries, 1);
    undo(&mut c);
    assert_eq!(c.model.index.layer("b").unwrap().from, 5.);
}
#[test]
fn preplanned_operation_batch_cannot_cross_caller_origin() {
    let (_a, mut c) = client();
    let p = c
        .model
        .move_tracks(
            &strings(&["b"]),
            1.,
            tracks::LaneDelta::default(),
            Origin::Interface,
        )
        .unwrap();
    c.caller = Caller::realm(510, 1, Origin::Mcp);
    assert!(matches!(
        block_on(c.dispatch(p)),
        Err(ClientError::OriginMismatch)
    ));
}
#[test]
fn preplanned_operations_are_bound_to_document_identity() {
    let (_a, mut c) = client();
    let p = c.model.reorder(&strings(&["c"]), "a", false, 0.).unwrap();
    block_on(c.kernel.close("P")).unwrap();
    block_on(c.kernel.open("P", OpenSource::Untitled(fixture())))
        .unwrap()
        .unwrap();
    assert!(matches!(
        block_on(c.dispatch(p)),
        Err(ClientError::DocumentChanged)
    ));
}
#[test]
fn selected_value_keys_edit_between_keys_without_inserting_a_new_one() {
    let (_a, mut c) = client();
    select_keys(&mut c, &["a0", "a2"]);
    block_on(c.begin_drag(DragKind::Property {
        targets: vec![("a".into(), "opacity".into())],
        time: 2.,
    }))
    .unwrap();
    block_on(c.drag(DragSample {
        value_delta: 5.,
        ..sample(0.)
    }))
    .unwrap();
    let ch = c.model.index.channel("a", "opacity").unwrap();
    assert_eq!(ch.keys.len(), 2);
    assert_eq!(ch.keys[0].value, Value::Number(15.));
    assert_eq!(ch.keys[1].value, Value::Number(55.));
    block_on(c.finish_drag(false)).unwrap();
}
#[test]
fn linked_scale_value_preserves_ratio_and_zero_baseline_rules() {
    let mut v = fixture();
    layer_mut(&mut v, 0).insert("scaleLinked", Value::Bool(true));
    let (_a, mut c) = client_with(v);
    let p = c
        .model
        .scale_value("a", "scale.x", 200., 2., 100., 50.)
        .unwrap();
    run(&mut c, p);
    let pubd = c.kernel.published("P").unwrap();
    let mut q = pm_kernel::QueryMirror::new(&pubd);
    assert_eq!(
        q.queries()
            .evaluate(Some("C"), "a", "scale.y", 2.)
            .unwrap()
            .as_f64(),
        Some(100.)
    );
    undo(&mut c);
    let p = c
        .model
        .scale_value("a", "scale.x", 20., 2., 0., 50.)
        .unwrap();
    run(&mut c, p);
    let pubd = c.kernel.published("P").unwrap();
    let mut q = pm_kernel::QueryMirror::new(&pubd);
    assert_eq!(
        q.queries()
            .evaluate(Some("C"), "a", "scale.y", 2.)
            .unwrap()
            .as_f64(),
        Some(20.)
    );
}
#[test]
fn property_target_translation_respects_mask_effect_ordinals_and_type() {
    let mut v = fixture();
    for (i, fx, mask) in [(0, "fxA", "maskA"), (1, "fxB", "maskB")] {
        let l = layer_mut(&mut v, i);
        l.insert(
            "fx",
            parse(&format!(
                r#"[{{"id":"{fx}","type":"blur","p":{{"radius":{{"v":5,"kf":[]}}}}}}]"#
            ))
            .unwrap(),
        );
        l.insert(
            "masks",
            parse(&format!(
                r#"[{{"id":"{mask}","p":{{"x":{{"v":0,"kf":[]}}}}}}]"#
            ))
            .unwrap(),
        );
    }
    let (_a, mut c) = client_with(v);
    select(&mut c, &["a", "b"]);
    assert_eq!(
        c.model
            .property_targets("a", &strings(&["fxA.radius", "m.maskA.x"]))
            .unwrap(),
        vec![
            ("a".into(), "fxA.radius".into()),
            ("a".into(), "m.maskA.x".into()),
            ("b".into(), "fxB.radius".into()),
            ("b".into(), "m.maskB.x".into())
        ]
    );
}
#[test]
fn shader_extension_transition_and_structured_channels_keep_contract_paths() {
    let mut v = fixture();
    let l = layer_mut(&mut v, 0);
    l.insert("type", Value::from("shader"));
    l.insert("d",parse(r#"{"uniforms":{"amount":{"v":1,"kf":[]}},"paths":[{"id":"path","p":{"closed":{"v":false,"kf":[]}},"vertices":[{"id":"vertex","p":{"x":{"v":2,"kf":[]}}}]}],"animators":[{"id":"animator","p":{"delay":{"v":0,"kf":[]}}}],"styles":[{"id":"style","p":{"size":{"v":16,"kf":[]}}}]}"#).unwrap());
    l.insert(
        "transitionIn",
        parse(r#"{"p":{"amount":{"v":1,"kf":[]}}}"#).unwrap(),
    );
    let m = model(v);
    for path in [
        "u.amount",
        "g.path.closed",
        "g.path.v.vertex.x",
        "ta.animator.delay",
        "ts.style.size",
        "transitionIn.p.amount",
    ] {
        assert!(m.index.channel("a", path).is_some(), "{path}");
    }
    assert!(m.index.channel("a", "c.uniforms.amount").is_none());
}
#[test]
fn parent_commands_are_atomic_and_reject_cycles() {
    let mut v = fixture();
    fill_transforms(&mut v);
    let (_a, mut c) = client_with(v);
    let p = c.model.parent(&strings(&["b"]), Some("a".into()), 0.);
    run(&mut c, p);
    assert_eq!(
        c.model.index.layer("b").unwrap().parent.as_deref(),
        Some("a")
    );
    let p = c.model.parent(&strings(&["a"]), Some("b".into()), 0.);
    assert!(block_on(c.dispatch(p)).is_err());
    undo(&mut c);
    assert!(c.model.index.layer("b").unwrap().parent.is_none());
}
#[test]
fn embedded_audio_mute_uses_content_transaction() {
    let mut v = fixture();
    layer_mut(&mut v, 0).insert("type", Value::from("video"));
    layer_mut(&mut v, 0).insert("d", parse(r#"{"audioMuted":false}"#).unwrap());
    let (_a, mut c) = client_with(v);
    let p = c.model.mute("a", 0.).unwrap();
    run(&mut c, p);
    assert!(c.model.index.layer("a").unwrap().audio_muted);
    undo(&mut c);
    assert!(!c.model.index.layer("a").unwrap().audio_muted);
}

fn fill_transforms(value: &mut Value) {
    for i in 0..3 {
        let l = layer_mut(value, i);
        let p = l.get_mut("p").unwrap().as_object_mut().unwrap();
        for (path, v) in [
            ("position.x", 0.),
            ("position.y", 0.),
            ("anchor.x", 0.),
            ("anchor.y", 0.),
            ("rotation", 0.),
            ("skew", 0.),
            ("scale.x", 100.),
            ("scale.y", 100.),
        ] {
            if p.get(path).is_none() {
                let mut prop = pm_model::Map::new();
                prop.insert("v", Value::Number(v));
                prop.insert("kf", Value::Array(Vec::new()));
                p.insert(path, Value::Object(prop));
            }
        }
    }
}
#[test]
fn scale_row_groups_axes_but_keeps_individual_stored_ids() {
    let m = model(fixture());
    assert!(
        m.rows()
            .iter()
            .any(|r| matches!(&r.id,RowId::Property { channel,.. } if channel=="scale"))
    );
    assert!(
        !m.rows()
            .iter()
            .any(|r| matches!(&r.id,RowId::Property { channel,.. } if channel=="scale.y"))
    );
    assert_eq!(m.track_channels("a", "scale").len(), 2);
}
#[test]
fn property_search_reveals_collapsed_group_children_without_document_changes() {
    let mut v = fixture();
    layer_mut(&mut v, 0).insert("type", Value::from("group"));
    layer_mut(&mut v, 0).insert("collapsed", Value::Bool(true));
    layer_mut(&mut v, 1).insert("group", Value::from("a"));
    let mut m = model(v);
    let revision = m.index.revision;
    assert!(m.row_position(&RowId::Layer("b".into())).is_none());
    m.set_search("B");
    assert!(m.row_position(&RowId::Layer("b".into())).is_some());
    m.set_search("");
    assert!(m.row_position(&RowId::Layer("b".into())).is_none());
    assert_eq!(m.index.revision, revision);
}
#[test]
fn external_selection_reveals_group_ancestors() {
    let mut v = fixture();
    layer_mut(&mut v, 0).insert("type", Value::from("group"));
    layer_mut(&mut v, 0).insert("collapsed", Value::Bool(true));
    layer_mut(&mut v, 1).insert("group", Value::from("a"));
    let (_a, mut c) = client_with(v);
    assert!(c.model.row_position(&RowId::Layer("b".into())).is_none());
    select(&mut c, &["b"]);
    assert!(c.model.row_position(&RowId::Layer("b".into())).is_some());
}
#[test]
fn keyframe_proximity_latch_matches_600_tag_generated_samples() {
    for case in oracle().get("groupSnapping").unwrap().as_array().unwrap() {
        let numbers = |key| {
            case.get(key)
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_f64().unwrap())
                .collect::<Vec<_>>()
        };
        let anchors = numbers("anchors");
        let targets = numbers("targets");
        let mut lock = None;
        for step in case.get("steps").unwrap().as_array().unwrap() {
            let (delta, next) =
                snap::keyframe_group(&anchors, n(step, "raw"), &targets, 0.1, lock, 0.15);
            lock = next;
            assert!((delta - n(step, "delta")).abs() < 1e-9);
            match step.get("lock").unwrap() {
                Value::Null => assert!(lock.is_none()),
                v => assert_eq!(
                    lock,
                    Some(snap::GroupLock {
                        anchor: n(v, "anchor"),
                        target: n(v, "target")
                    })
                ),
            }
        }
    }
}
#[test]
fn playhead_targets_match_twenty_tag_frame_boundary_cases() {
    for case in oracle().get("playhead").unwrap().as_array().unwrap() {
        let input = case.get("input").unwrap();
        let mut value = fixture();
        let map = value.as_object_mut().unwrap();
        map.insert("fps", input.get("fps").unwrap().clone());
        map.insert("work", input.get("work").unwrap().clone());
        map.insert("markers", input.get("markers").unwrap().clone());
        let layers = input
            .get("layers")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let mut layer = l.as_object().unwrap().clone();
                layer.remove("keys");
                layer.insert("id", Value::from(format!("L{i}")));
                layer.insert("type", Value::from("shape"));
                let mut keys = Vec::new();
                for (j, t) in l
                    .get("keys")
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    let mut k = pm_model::Map::new();
                    k.insert("i", Value::from(format!("K{i}_{j}")));
                    k.insert("t", t.clone());
                    k.insert("v", Value::Number(0.));
                    keys.push(Value::Object(k));
                }
                let mut prop = pm_model::Map::new();
                prop.insert("v", Value::Number(0.));
                prop.insert("kf", Value::Array(keys));
                let mut p = pm_model::Map::new();
                p.insert("opacity", Value::Object(prop));
                layer.insert("p", Value::Object(p));
                Value::Object(layer)
            })
            .collect();
        map.insert("layers", Value::Array(layers));
        let m = model(value);
        let actual = playhead_targets(&m.index);
        let want = case.get("expected").unwrap().as_array().unwrap();
        assert_eq!(actual.len(), want.len());
        for (a, b) in actual.iter().zip(want) {
            assert!((a - b.as_f64().unwrap()).abs() < 1e-9);
        }
    }
}
#[test]
fn linked_hit_with_one_selected_axis_preserves_selection_on_click() {
    let (_a, mut c) = client();
    select_keys(&mut c, &["sx"]);
    let calls = c
        .model
        .select_keys("a", "scale", &strings(&["sx", "sy"]), false, false)
        .unwrap();
    run(
        &mut c,
        Plan {
            calls,
            ..Default::default()
        },
    );
    assert_eq!(c.model.selection.keys(), strings(&["sx", "sy"]));
}
#[test]
fn shift_marquee_toggles_keys_against_captured_selection_each_sample() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    select_keys(&mut c, &["a0"]);
    let mut marquee = MarqueeSession::begin(&c.model, true, false);
    for _ in 0..2 {
        let p = marquee.update(&c.model, strings(&["a0", "a2"]), Vec::new());
        run(&mut c, p);
        assert_eq!(c.model.selection.keys(), strings(&["a2"]));
    }
}
#[test]
fn graph_marquee_retains_original_during_preview_then_replaces_on_release() {
    let (_a, mut c) = client();
    select_keys(&mut c, &["a0"]);
    let mut marquee = MarqueeSession::begin(&c.model, false, true);
    let p = marquee.update(&c.model, strings(&["a2"]), Vec::new());
    run(&mut c, p);
    assert_eq!(c.model.selection.keys(), strings(&["a0", "a2"]));
    let p = marquee.finish(&c.model);
    run(&mut c, p);
    assert_eq!(c.model.selection.keys(), strings(&["a2"]));
}
#[test]
fn graph_marquee_empty_release_and_escape_restore_original_ids() {
    let (_a, mut c) = client();
    select(&mut c, &["a"]);
    select_keys(&mut c, &["a0"]);
    let mut marquee = MarqueeSession::begin(&c.model, false, true);
    let p = marquee.update(&c.model, Vec::new(), Vec::new());
    run(&mut c, p);
    let p = marquee.finish(&c.model);
    run(&mut c, p);
    assert_eq!(c.model.selection.keys(), strings(&["a0"]));
    let p = marquee.update(&c.model, strings(&["a2"]), Vec::new());
    run(&mut c, p);
    run(&mut c, marquee.cancel());
    assert_eq!(c.model.selection.keys(), strings(&["a0"]));
}
#[test]
fn graph_focus_resolution_precedes_selected_layer_and_falls_back_safely() {
    let candidates = vec![
        ("a".into(), "opacity".into()),
        ("b".into(), "opacity".into()),
    ];
    assert_eq!(
        graph_target(
            &candidates,
            Some(("a", "opacity")),
            &strings(&["b"]),
            Some("opacity")
        ),
        Some(&candidates[0])
    );
    assert_eq!(
        graph_target(&candidates, None, &strings(&["b"]), Some("opacity")),
        Some(&candidates[1])
    );
    assert!(graph_target(&[], None, &[], None).is_none());
}
#[test]
fn relative_key_gesture_refuses_to_compound_across_a_foreign_write() {
    let (_a, mut c) = client();
    block_on(c.begin_drag(DragKind::Keys {
        ids: strings(&["a0"]),
    }))
    .unwrap();
    block_on(c.drag(sample(0.5))).unwrap();
    let foreign = Caller::realm(777, 1, Origin::Agent);
    let call = Call::Transact {
        label: None,
        body: WriteBody::Commands(vec![Command::SetLayerName {
            layer_id: "a".into(),
            name: "Kept".into(),
        }]),
        expected_revision: None,
    };
    block_on(c.kernel.call("P", Request::new(foreign, call)))
        .unwrap()
        .unwrap();
    assert!(matches!(
        block_on(c.drag(sample(1.))),
        Err(ClientError::ConcurrentRelativeGesture)
    ));
    block_on(c.finish_drag(false)).unwrap();
    assert_eq!(key_time(&c, "a0"), 0.);
    assert_eq!(
        c.model.index.layer("a").unwrap().name.to_string_lossy(),
        "Kept"
    );
}
#[test]
fn async_disposal_cancels_owned_drag_without_blocking_in_drop() {
    let (_a, mut c) = client();
    let kernel = c.kernel.clone();
    block_on(c.begin_drag(DragKind::Layers {
        ids: strings(&["a"]),
    }))
    .unwrap();
    block_on(c.drag(sample(2.))).unwrap();
    assert!(block_on(c.dispose()).unwrap().is_some());
    let p = kernel.published("P").unwrap();
    let index = TimelineIndex::from_snapshot(&p.snapshot);
    assert_eq!(index.layer("a").unwrap().from, 1.);
    assert!(p.entry.is_none());
}
