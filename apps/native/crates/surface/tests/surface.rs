use pm_surface::{binding::*, controls::*, interaction::*, style::*, *};
use serde_json::{Value, json};
fn apply(s: &mut Surface, ops: Value) -> Result<Vec<Diagnostic>, Rejected> {
    s.apply_json(&json!({"baseRevision":s.revision(),"ops":ops}).to_string())
}
fn surface() -> Surface {
    let mut s = Surface::default();
    apply(
        &mut s,
        json!([
            {"op":"create","id":1,"type":"div"},
            {"op":"create","id":2,"type":"input"},
            {"op":"create","id":3,"type":"button"},
            {"op":"insertBefore","parent":0,"id":1,"before":null},
            {"op":"insertBefore","parent":1,"id":2,"before":null},
            {"op":"insertBefore","parent":1,"id":3,"before":null}
        ]),
    )
    .unwrap();
    s
}
#[test]
fn wire_all_six_ops_and_inspection() {
    let mut s = surface();
    apply(
        &mut s,
        json!([
            {"op":"setText","id":3,"text":"Run"},
            {"op":"setProps","id":2,"patch":{"value":"a","binding":"position"}},
            {"op":"setListeners","id":3,"flags":{"click":{"bubble":true}}},
            {"op":"insertBefore","parent":1,"id":3,"before":2}
        ]),
    )
    .unwrap();
    assert_eq!(s.node(1).unwrap().children, vec![3, 2]);
    assert_eq!(s.node(3).unwrap().text, "Run");
    apply(&mut s, json!([{"op":"remove","id":1}])).unwrap();
    assert_eq!(s.nodes().len(), 1);
}
#[test]
fn every_primitive_has_a_headless_representation() {
    for kind in PRIMITIVES {
        let mut s = Surface::default();
        s.allow_webview = true;
        apply(&mut s,json!([{"op":"create","id":1,"type":kind},{"op":"insertBefore","parent":0,"id":1,"before":null}])).unwrap();
        assert_eq!(s.node(1).unwrap().r#type, *kind);
    }
}
#[test]
fn invalid_tail_rolls_back_tree_input_and_revision() {
    let mut s = surface();
    let before = serde_json::to_value(s.nodes()).unwrap();
    let rev = s.revision();
    let error = apply(
        &mut s,
        json!([{"op":"setProps","id":2,"patch":{"value":"bad"}},{"op":"remove","id":999}]),
    )
    .unwrap_err();
    assert_eq!(error.op, Some(1));
    assert_eq!(s.revision(), rev);
    assert_eq!(serde_json::to_value(s.nodes()).unwrap(), before);
}
#[test]
fn invalid_commit_does_not_consume_new_ids() {
    let mut s = surface();
    assert!(apply(&mut s, json!([{"op":"create","id":4,"type":"div"}])).is_err());
    apply(&mut s,json!([{"op":"create","id":4,"type":"div"},{"op":"insertBefore","parent":0,"id":4,"before":null}])).unwrap();
}
#[test]
fn removed_ids_never_alias_queued_events() {
    let mut s = surface();
    apply(&mut s, json!([{"op":"remove","id":3}])).unwrap();
    assert!(apply(&mut s, json!([{"op":"create","id":3,"type":"button"}])).is_err());
}
#[test]
fn root_self_cycle_and_bad_before_rejected() {
    let cases = json!([
        [{"op":"remove","id":0}],
        [{"op":"insertBefore","parent":1,"id":1,"before":null}],
        [{"op":"insertBefore","parent":2,"id":1,"before":null}],
        [{"op":"insertBefore","parent":0,"id":1,"before":2}],
        [{"op":"insertBefore","parent":1,"id":0,"before":null}]
    ]);
    for ops in cases.as_array().unwrap() {
        assert!(apply(&mut surface(), ops.clone()).is_err());
    }
}
#[test]
fn temporary_detachment_is_allowed_but_final_orphans_rejected() {
    let mut s = surface();
    apply(&mut s,json!([{"op":"create","id":4,"type":"div"},{"op":"insertBefore","parent":4,"id":3,"before":null},{"op":"insertBefore","parent":0,"id":4,"before":null}])).unwrap();
    assert_eq!(s.node(3).unwrap().parent, Some(4));
}
#[test]
fn insert_before_self_is_noop_and_moves_do_not_duplicate() {
    let mut s = surface();
    apply(&mut s,json!([{"op":"insertBefore","parent":1,"id":2,"before":2},{"op":"insertBefore","parent":1,"id":2,"before":null}])).unwrap();
    assert_eq!(s.node(1).unwrap().children, vec![3, 2]);
}
#[test]
fn unknown_props_warn_and_are_retained_null_removes() {
    let mut s = surface();
    let d = apply(
        &mut s,
        json!([{"op":"setProps","id":3,"patch":{"futureFeature":{"exact":1},"disabled":false}}]),
    )
    .unwrap();
    assert_eq!(d[0].code, "unknownProp");
    assert_eq!(
        s.node(3).unwrap().props["futureFeature"],
        json!({"exact":1})
    );
    apply(
        &mut s,
        json!([{"op":"setProps","id":3,"patch":{"futureFeature":null}}]),
    )
    .unwrap();
    assert!(!s.node(3).unwrap().props.contains_key("futureFeature"));
}
#[test]
fn malformed_known_props_and_wire_fields_rejected() {
    let mut s = surface();
    for patch in [
        json!({"disabled":"yes"}),
        json!({"style":{"display":"block"}}),
        json!({"binding":12}),
        json!({"value":99}),
    ] {
        assert!(apply(&mut s, json!([{"op":"setProps","id":2,"patch":patch}])).is_err());
    }
    assert!(
        s.apply_json(r#"{"baseRevision":1,"ops":[],"extra":true}"#)
            .is_err()
    );
    assert!(
        s.apply_json(r#"{"baseRevision":1,"ops":[{"op":"remove","id":2,"extra":true}]}"#)
            .is_err()
    );
}
#[test]
fn stale_revision_is_rejected() {
    let mut s = surface();
    assert!(s.apply_json(r#"{"baseRevision":0,"ops":[]}"#).is_err());
    assert_eq!(s.revision(), 1);
}
#[test]
fn node_op_depth_byte_and_safe_integer_limits() {
    let mut s = surface();
    s.limits.ops = 0;
    assert!(apply(&mut s, json!([{"op":"remove","id":3}])).is_err());
    let mut s = surface();
    s.limits.nodes = 4;
    assert!(apply(&mut s, json!([{"op":"create","id":4,"type":"div"}])).is_err());
    let mut s = surface();
    s.limits.depth = 1;
    assert!(apply(&mut s, json!([])).is_err());
    let mut s = surface();
    s.limits.bytes = 2;
    assert!(s.apply_json("{} ").is_err());
    assert!(
        apply(
            &mut surface(),
            json!([{"op":"create","id":9007199254740992u64,"type":"div"}])
        )
        .is_err()
    );
}
#[test]
fn webview_is_explicitly_host_opt_in() {
    assert!(
        apply(
            &mut surface(),
            json!([{"op":"create","id":4,"type":"webview"}])
        )
        .is_err()
    );
}
#[test]
fn set_text_never_silently_erases_children() {
    assert!(apply(&mut surface(), json!([{"op":"setText","id":1,"text":"x"}])).is_err());
}
#[test]
fn controlled_input_echo_preserves_ime_caret_and_native_write_count() {
    let mut s = surface();
    s.observe_input(2, "a😀b".into(), (3, 3), true).unwrap();
    apply(
        &mut s,
        json!([{"op":"setProps","id":2,"patch":{"value":"a😀b"}}]),
    )
    .unwrap();
    let i = s.node(2).unwrap().input.as_ref().unwrap();
    assert_eq!(i.selection, (3, 3));
    assert!(i.composing);
    assert_eq!(i.writes, 0);
    apply(
        &mut s,
        json!([{"op":"setProps","id":2,"patch":{"value":"z"}}]),
    )
    .unwrap();
    let i = s.node(2).unwrap().input.as_ref().unwrap();
    assert_eq!(i.displayed, "z");
    assert_eq!(i.selection, (1, 1));
    assert_eq!(i.writes, 1);
    assert!(s.observe_input(2, "x".into(), (2, 2), false).is_err());
}
#[test]
fn numeric_native_inputs_accept_typed_document_numbers() {
    let mut s = surface();
    apply(
        &mut s,
        json!([{"op":"setProps","id":2,"patch":{"control":"number","value":17}}]),
    )
    .unwrap();
    assert_eq!(s.node(2).unwrap().input.as_ref().unwrap().displayed, "17");
}
#[test]
fn event_capture_target_bubble_defaults_and_key_filters() {
    let mut s = surface();
    apply(&mut s,json!([
        {"op":"setListeners","id":1,"flags":{"keydown":{"capture":true,"bubble":true}}},
        {"op":"setListeners","id":2,"flags":{"keydown":{"capture":true,"bubble":true,"preventDefault":true,"keys":["Escape"]}}}
    ])).unwrap();
    let i = Interaction::default();
    let plan = i.plan(&s, 2, None, "keydown", Some("Escape")).unwrap();
    assert_eq!(
        plan.deliveries,
        vec![
            Delivery {
                id: 1,
                capture: true
            },
            Delivery {
                id: 2,
                capture: true
            },
            Delivery {
                id: 2,
                capture: false
            },
            Delivery {
                id: 1,
                capture: false
            }
        ]
    );
    assert!(plan.prevent_default);
    assert!(
        !i.plan(&s, 2, None, "keydown", Some("a"))
            .unwrap()
            .prevent_default
    );
}
#[test]
fn pointer_capture_release_and_removal() {
    let mut s = surface();
    let mut i = Interaction::default();
    i.capture(&s, 5, 2).unwrap();
    assert_eq!(
        i.plan(&s, 3, Some(5), "pointermove", None).unwrap().target,
        2
    );
    i.release(5);
    assert_eq!(
        i.plan(&s, 3, Some(5), "pointermove", None).unwrap().target,
        3
    );
    i.capture(&s, 5, 2).unwrap();
    apply(&mut s, json!([{"op":"remove","id":2}])).unwrap();
    i.reconcile(&s);
    assert_eq!(i.captured(5), None);
}
#[test]
fn drag_threshold_is_three_points_and_stays_active() {
    let mut d = Drag::new(0., 0.);
    assert_eq!(d.update(2., 2.), None);
    assert_eq!(d.update(3., 0.), Some((3., 0.)));
    assert_eq!(d.update(1., 0.), Some((1., 0.)));
    assert_eq!(d.update(f64::NAN, 0.), None);
}
#[test]
fn focus_scope_traps_tab_and_restores_previous_focus() {
    let mut s = surface();
    let mut i = Interaction::default();
    i.focus(&s, 3).unwrap();
    apply(&mut s,json!([{"op":"setProps","id":1,"patch":{"focusScope":true}},{"op":"setProps","id":3,"patch":{"disabled":true}}])).unwrap();
    i.push_scope(&s, 1).unwrap();
    assert_eq!(i.tab(&s, false).unwrap(), Some(2));
    assert_eq!(i.tab(&s, false).unwrap(), Some(2));
    assert!(i.focus(&s, 0).is_err());
    assert!(i.focus(&s, 3).is_err());
    i.pop_scope(&s);
    assert_eq!(i.focused, None);
}
#[test]
fn selectors_cascade_specificity_source_order_inline_and_token_data() {
    let mut s = surface();
    apply(&mut s,json!([{"op":"setProps","id":3,"patch":{"id":"run","class":"primary other","role":"button","attr":{"kind":"edit"},"state":{"hover":true},"style":{"gap":{"token":"metrics.gap"}}}}])).unwrap();
    let node = s.node(3).unwrap();
    let selectors = vec![
        Selector::Id("run".into()),
        Selector::Class("primary".into()),
        Selector::Role("button".into()),
        Selector::Attr {
            name: "kind".into(),
            value: json!("edit"),
        },
        Selector::State {
            name: "hover".into(),
            value: json!(true),
        },
    ];
    assert!(Selector::All(selectors.clone()).matches(node));
    let mut sheet = StyleSheet::default();
    sheet
        .replace(vec![
            Rule {
                selector: selectors[0].clone(),
                style: json!({"display":"grid","gap":1})
                    .as_object()
                    .unwrap()
                    .clone(),
            },
            Rule {
                selector: selectors[1].clone(),
                style: json!({"display":"flex"}).as_object().unwrap().clone(),
            },
        ])
        .unwrap();
    let style = sheet.resolve(node);
    assert_eq!(style["display"], "grid");
    assert_eq!(style["gap"], json!({"token":"metrics.gap"}));
    assert!(
        sheet
            .replace(vec![Rule {
                selector: selectors[0].clone(),
                style: json!({"display":"block"}).as_object().unwrap().clone()
            }])
            .is_err()
    );
    assert_eq!(sheet.resolve(node), style);
}
#[derive(Default)]
struct Sink {
    edits: Vec<Edit>,
    fail: bool,
}
impl BindingSink for Sink {
    fn enqueue(&mut self, e: Edit) -> Result<(), String> {
        if self.fail {
            return Err("queue full".into());
        }
        self.edits.push(e);
        Ok(())
    }
}
fn bound(control: &str, value: Value) -> Surface {
    let mut s = surface();
    apply(
        &mut s,
        json!([{"op":"setProps","id":2,"patch":{"control":control,"binding":"b","value":value}}]),
    )
    .unwrap();
    s
}
#[test]
fn binding_begin_write_commit_and_no_double_begin() {
    let s = bound("text", json!("x"));
    let mut b = Bindings::default();
    let mut sink = Sink::default();
    b.begin(&s, 2, &mut sink).unwrap();
    assert!(b.begin(&s, 2, &mut sink).is_err());
    b.write(2, json!("y"), &mut sink).unwrap();
    b.finish(2, false, &mut sink).unwrap();
    assert_eq!(sink.edits.len(), 3);
    assert!(matches!(&sink.edits[2],Edit::Commit{binding} if binding=="b"));
}
#[test]
fn binding_rebind_or_destroy_cancels_original_and_queue_failures_retry() {
    let mut s = bound("text", json!("x"));
    let mut b = Bindings::default();
    let mut sink = Sink::default();
    b.begin(&s, 2, &mut sink).unwrap();
    apply(
        &mut s,
        json!([{"op":"setProps","id":2,"patch":{"binding":"new"}}]),
    )
    .unwrap();
    sink.fail = true;
    assert!(b.reconcile(&s, &mut sink).is_err());
    sink.fail = false;
    b.reconcile(&s, &mut sink).unwrap();
    assert_eq!(
        sink.edits[1],
        Edit::Cancel {
            binding: "b".into()
        }
    );
}
#[test]
fn text_control_live_input_escape_and_blur_transactions() {
    let s = bound("text", json!("before"));
    let mut c = Controls::default();
    let mut sink = Sink::default();
    c.focus(&s, 2, &mut sink).unwrap();
    c.input(2, "after".into(), &mut sink).unwrap();
    c.finish(&s, 2, true, &mut sink).unwrap();
    assert!(matches!(sink.edits[2], Edit::Cancel { .. }));
    c.focus(&s, 2, &mut sink).unwrap();
    c.finish(&s, 2, false, &mut sink).unwrap();
    assert!(matches!(sink.edits.last(), Some(Edit::Commit { .. })));
}
#[test]
fn numeric_typing_is_changed_finite_only_and_not_scrub_clamped() {
    let mut s = bound("number", json!(-2));
    apply(
        &mut s,
        json!([{"op":"setProps","id":2,"patch":{"min":0,"max":10}}]),
    )
    .unwrap();
    let mut c = Controls::default();
    let mut sink = Sink::default();
    for raw in ["-2", "nope", "1/0"] {
        c.focus(&s, 2, &mut sink).unwrap();
        c.input(2, raw.into(), &mut sink).unwrap();
        c.finish(&s, 2, false, &mut sink).unwrap();
    }
    assert!(sink.edits.is_empty());
    c.focus(&s, 2, &mut sink).unwrap();
    c.input(2, "99".into(), &mut sink).unwrap();
    c.finish(&s, 2, false, &mut sink).unwrap();
    assert_eq!(
        sink.edits[1],
        Edit::Write {
            binding: "b".into(),
            value: json!(99.0)
        }
    );
}
#[test]
fn arithmetic_relative_absolute_negative_parentheses_and_reject_source() {
    for (raw, expected) in [
        ("+5", 15.),
        ("*2", 20.),
        ("/4", 2.5),
        ("-=3", 7.),
        ("-2", -2.),
        ("(2+3)*4", 20.),
        ("1e2 / 4", 25.),
        ("--2", 2.),
    ] {
        assert_eq!(numeric_draft(raw, 10.), Some(expected), "{raw}");
    }
    for raw in ["", "alert(1)", "1;2", "1/0", "1e309", "NaN"] {
        assert_eq!(numeric_draft(raw, 10.), None);
    }
}
#[test]
fn numeric_arrows_clamp_and_drafts_defer_document_writes() {
    let mut s = bound("number", json!(2));
    apply(
        &mut s,
        json!([{"op":"setProps","id":2,"patch":{"min":0,"max":5,"step":1}}]),
    )
    .unwrap();
    let mut c = Controls::default();
    let mut sink = Sink::default();
    c.numeric_key(&s, 2, "ArrowUp", true, &mut sink).unwrap();
    assert_eq!(
        sink.edits[1],
        Edit::Write {
            binding: "b".into(),
            value: json!(5.0)
        }
    );
    sink.edits.clear();
    c.focus(&s, 2, &mut sink).unwrap();
    c.numeric_key(&s, 2, "ArrowDown", false, &mut sink).unwrap();
    assert!(sink.edits.is_empty());
    c.finish(&s, 2, false, &mut sink).unwrap();
    assert_eq!(sink.edits.len(), 3);
}
#[test]
fn scrub_direction_modifiers_bounds_and_finite_results() {
    assert_eq!(
        scrub(Scrub {
            start: 10.,
            delta: (2., 0.),
            step: 1.,
            speed: 0.5,
            shift: false,
            alt: false,
            bounds: (0., 100.)
        })
        .unwrap(),
        11.
    );
    assert_eq!(
        scrub(Scrub {
            start: 10.,
            delta: (0., -2.),
            step: 1.,
            speed: 0.5,
            shift: true,
            alt: false,
            bounds: (0., 100.)
        })
        .unwrap(),
        20.
    );
    assert_eq!(
        scrub(Scrub {
            start: 10.,
            delta: (2., 0.),
            step: 1.,
            speed: 0.5,
            shift: false,
            alt: true,
            bounds: (0., 100.)
        })
        .unwrap(),
        10.1
    );
    assert!(
        scrub(Scrub {
            start: 0.,
            delta: (f64::NAN, 0.),
            step: 1.,
            speed: 1.,
            shift: false,
            alt: false,
            bounds: (0., 1.)
        })
        .is_err()
    );
}
#[test]
fn typed_select_values_and_ui_only_decline() {
    let mut s = bound("select", json!("a"));
    apply(&mut s,json!([{"op":"setProps","id":2,"patch":{"options":["a",{"v":{"exact":[1,true]},"label":"object"}]}}])).unwrap();
    let mut c = Controls::default();
    let mut sink = Sink::default();
    c.select(&s, 2, 1, true, &mut sink).unwrap();
    assert!(sink.edits.is_empty());
    c.select(&s, 2, 1, false, &mut sink).unwrap();
    assert_eq!(
        sink.edits[1],
        Edit::Write {
            binding: "b".into(),
            value: json!({"exact":[1,true]})
        }
    );
}
#[test]
fn toggle_mixed_and_unchanged_values() {
    let mut s = bound("text", json!(""));
    apply(
        &mut s,
        json!([{"op":"setProps","id":3,"patch":{"binding":"toggle","value":false,"mixed":true}}]),
    )
    .unwrap();
    let mut c = Controls::default();
    let mut sink = Sink::default();
    c.toggle(&s, 3, "Enter", &mut sink).unwrap();
    assert_eq!(
        sink.edits[1],
        Edit::Write {
            binding: "toggle".into(),
            value: json!(true)
        }
    );
}
#[test]
fn accepted_publication_has_a_linux_host_double() {
    use std::sync::Arc;
    let s = surface();
    let mut host = InspectHost::default();
    host.enqueue(Arc::new(s), vec![]);
    assert_eq!(host.tree.unwrap().node(2).unwrap().r#type, "input");
}
#[test]
fn thousand_node_commit_and_reorder_are_atomic() {
    let mut s = Surface::default();
    let mut ops = vec![];
    for id in 1..=1000 {
        ops.push(json!({"op":"create","id":id,"type":"button"}));
        ops.push(json!({"op":"insertBefore","parent":0,"id":id,"before":null}));
    }
    apply(&mut s, json!(ops)).unwrap();
    let mut ops = vec![];
    for id in (1..=1000).rev() {
        ops.push(json!({"op":"insertBefore","parent":0,"id":id,"before":null}));
    }
    apply(&mut s, json!(ops)).unwrap();
    assert_eq!(
        s.node(0).unwrap().children,
        (1..=1000).rev().collect::<Vec<_>>()
    );
}
#[test]
fn stylesheet_replacement_is_in_the_same_atomic_commit_as_tree_ops() {
    let mut s = surface();
    apply(&mut s,json!([
        {"op":"setProps","id":3,"patch":{"class":"primary"}},
        {"op":"setProps","id":0,"patch":{"styleRules":[{"selector":{"kind":"class","value":"primary"},"style":{"display":"grid"}}]}}
    ])).unwrap();
    assert_eq!(s.computed_style(3).unwrap()["display"], "grid");
    assert!(
        apply(
            &mut s,
            json!([
                {"op":"setProps","id":0,"patch":{"styleRules":[]}},
                {"op":"remove","id":999}
            ])
        )
        .is_err()
    );
    assert_eq!(s.computed_style(3).unwrap()["display"], "grid");
}
#[test]
fn rebinding_a_numeric_draft_never_commits_to_the_replacement_binding() {
    let mut s = bound("number", json!(1));
    let mut c = Controls::default();
    let mut sink = Sink::default();
    c.focus(&s, 2, &mut sink).unwrap();
    c.input(2, "99".into(), &mut sink).unwrap();
    apply(
        &mut s,
        json!([{"op":"setProps","id":2,"patch":{"binding":"replacement"}}]),
    )
    .unwrap();
    c.reconcile(&s, &mut sink).unwrap();
    c.finish(&s, 2, false, &mut sink).unwrap();
    assert!(sink.edits.is_empty());
}
#[test]
fn removing_a_focus_scope_restores_an_existing_enabled_target() {
    let mut s = surface();
    let mut i = Interaction::default();
    i.focus(&s, 3).unwrap();
    apply(&mut s,json!([{"op":"setProps","id":1,"patch":{"focusScope":true}},{"op":"insertBefore","parent":0,"id":3,"before":null}])).unwrap();
    i.push_scope(&s, 1).unwrap();
    i.focus(&s, 2).unwrap();
    apply(&mut s, json!([{"op":"remove","id":1}])).unwrap();
    i.reconcile(&s);
    assert_eq!(i.focused, Some(3));
}
