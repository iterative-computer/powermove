#[path = "pro_fixture.rs"]
mod fixture;
use fixture::*;
use pm_kernel::commands::pro_edit::{Edge, ProEditCommand as Edit, SplitTail, TrackTarget};
use pm_kernel::commands::pro_links::{ProLinkCommand as Link, linked_selection, sync_badges};
use pm_kernel::{Command, Origin};
use serde_json::json;
fn tracks() -> Vec<TrackTarget> {
    vec![TrackTarget::new(0)]
}
fn link(s: &mut pm_kernel::DocumentStore) {
    run(
        s,
        Link::Link {
            video_id: "b".into(),
            audio_id: "sound".into(),
            link_id: "pair".into(),
        },
    );
}
fn paired() -> pm_kernel::DocumentStore {
    let mut s = store(vec![
        clip("a", 0., 4., 0),
        clip("b", 4., 4., 0),
        clip("c", 8., 4., 0),
        audio("sound", 4., 4., 1),
    ]);
    link(&mut s);
    s
}
fn roll(delta: f64) -> Edit {
    Edit::Roll {
        left_id: "a".into(),
        right_id: "b".into(),
        delta,
        linked: true,
    }
}
fn slide(delta: f64) -> Edit {
    Edit::Slide {
        previous_id: "a".into(),
        layer_id: "b".into(),
        next_id: "c".into(),
        delta,
        linked: true,
    }
}
#[test]
fn roll_preserves_outer_bounds_source_and_absolute_keys() {
    let mut s = abc();
    roundtrip(&mut s, roll(1.));
    assert_eq!(n(&layer(&s, "a"), &["dur"]), 5.);
    let b = layer(&s, "b");
    assert_eq!(n(&b, &["from"]), 5.);
    assert_eq!(n(&b, &["dur"]), 3.);
    assert_eq!(n(&b, &["d", "trim", "v"]), 7.);
    assert_eq!(n(&b, &["p", "position.x", "kf", "0", "t"]), 0.);
    assert!(b.get("extensionData").is_some());
}
#[test]
fn slide_compensates_neighbors_and_keeps_middle_source_and_animation() {
    let mut s = abc();
    let before = layer(&s, "b");
    roundtrip(&mut s, slide(1.));
    let b = layer(&s, "b");
    assert_eq!(n(&b, &["from"]), 5.);
    assert_eq!(b.get("d"), before.get("d"));
    assert_eq!(b.get("p"), before.get("p"));
    assert_eq!(n(&layer(&s, "a"), &["dur"]), 5.);
    assert_eq!(n(&layer(&s, "c"), &["from"]), 9.);
    assert_eq!(n(&layer(&s, "c"), &["dur"]), 3.);
}
#[test]
fn ripple_delete_closes_gap_on_explicit_tracks() {
    let mut s = abc();
    roundtrip(
        &mut s,
        Edit::RippleDelete {
            layer_ids: ids(&["b"]),
            tracks: tracks(),
            linked: true,
        },
    );
    assert_eq!(n(&layer(&s, "c"), &["from"]), 4.);
    assert!(pm_kernel::model::find_layer(s.root().unwrap(), "b").is_none());
}
#[test]
fn ripple_delete_multiple_adjacent_ranges_and_parallel_pairs() {
    let mut s = paired();
    roundtrip(
        &mut s,
        Edit::RippleDelete {
            layer_ids: ids(&["a", "b"]),
            tracks: vec![TrackTarget::new(0), TrackTarget::new(1)],
            linked: true,
        },
    );
    assert_eq!(n(&layer(&s, "c"), &["from"]), 0.);
}
#[test]
fn ripple_delete_rejects_disjoint_selection_and_crossing_survivor() {
    let mut s = abc();
    reject(
        &mut s,
        Edit::RippleDelete {
            layer_ids: ids(&["a", "c"]),
            tracks: tracks(),
            linked: true,
        },
    );
    let mut s = store(vec![
        clip("a", 0., 5., 0),
        clip("b", 4., 4., 0),
        clip("c", 8., 4., 0),
    ]);
    reject(
        &mut s,
        Edit::RippleDelete {
            layer_ids: ids(&["b"]),
            tracks: tracks(),
            linked: true,
        },
    );
}
#[test]
fn ripple_in_trim_keeps_in_point_and_rebases_absolute_keys() {
    let mut s = abc();
    roundtrip(
        &mut s,
        Edit::RippleTrim {
            layer_id: "a".into(),
            edge: Edge::In,
            delta: 1.,
            tracks: tracks(),
            linked: true,
        },
    );
    let a = layer(&s, "a");
    assert_eq!(n(&a, &["from"]), 0.);
    assert_eq!(n(&a, &["dur"]), 3.);
    assert_eq!(n(&a, &["d", "trim", "v"]), 3.);
    assert_eq!(n(&a, &["p", "position.x", "kf", "0", "t"]), 0.);
    assert_eq!(n(&layer(&s, "b"), &["from"]), 3.);
    assert_eq!(n(&layer(&s, "c"), &["from"]), 7.);
}
#[test]
fn ripple_out_trim_opens_and_closes_exact_delta() {
    for delta in [-1., 1.] {
        let mut s = abc();
        roundtrip(
            &mut s,
            Edit::RippleTrim {
                layer_id: "a".into(),
                edge: Edge::Out,
                delta,
                tracks: tracks(),
                linked: true,
            },
        );
        assert_eq!(n(&layer(&s, "b"), &["from"]), 4. + delta);
        assert_eq!(n(&layer(&s, "c"), &["from"]), 8. + delta);
    }
}
#[test]
fn locked_downstream_refuses_entire_ripple_and_locked_neighbor_refuses_roll() {
    let mut c = clip("c", 8., 4., 0);
    c["lock"] = json!(true);
    let mut s = store(vec![clip("a", 0., 4., 0), clip("b", 4., 4., 0), c]);
    reject(
        &mut s,
        Edit::RippleTrim {
            layer_id: "a".into(),
            edge: Edge::Out,
            delta: 1.,
            tracks: tracks(),
            linked: true,
        },
    );
    let mut b = clip("b", 4., 4., 0);
    b["lock"] = json!(true);
    let mut s = store(vec![clip("a", 0., 4., 0), b]);
    reject(&mut s, roll(1.));
}
#[test]
fn exhausted_handles_minimum_duration_nan_and_nonadjacency_refuse() {
    for d in [-20., 4., f64::NAN, f64::INFINITY] {
        let mut s = abc();
        reject(&mut s, roll(d));
    }
    let mut s = store(vec![clip("a", 0., 3., 0), clip("b", 4., 4., 0)]);
    reject(&mut s, roll(1.));
    let mut a = clip("a", 0., 4., 0);
    a["d"]["trim"]["v"] = json!(96.);
    let mut s = store(vec![a, clip("b", 4., 4., 0)]);
    reject(&mut s, roll(1.));
}
#[test]
fn locked_ancestor_and_animated_timing_are_atomic() {
    let mut b = clip("b", 4., 4., 0);
    b["group"] = json!("g");
    let mut a = clip("a", 0., 4., 0);
    a["group"] = json!("g");
    let g = json!({"id":"g","type":"group","lock":true,"from":0,"dur":12});
    let mut s = store(vec![g, a, b]);
    reject(&mut s, roll(1.));
    for key in ["trim", "speed", "sourceTime", "timeRemap"] {
        let mut b = clip("b", 4., 4., 0);
        b["d"][key] = json!({"v":1,"kf":[{"t":0,"v":1,"i":"timing"}],"expr":null});
        let mut s = store(vec![b]);
        reject(
            &mut s,
            Edit::Slip {
                layer_ids: ids(&["b"]),
                delta: 1.,
                linked: true,
            },
        );
    }
}
#[test]
fn slip_moves_source_by_rate_without_changing_strip_or_animation() {
    let mut b = clip("b", 4., 4., 0);
    b["d"]["speed"] = json!(2.);
    let mut s = store(vec![b]);
    let before = layer(&s, "b");
    roundtrip(
        &mut s,
        Edit::Slip {
            layer_ids: ids(&["b"]),
            delta: 0.5,
            linked: true,
        },
    );
    let after = layer(&s, "b");
    assert_eq!(n(&after, &["d", "trim", "v"]), 7.);
    for key in ["from", "dur", "p", "extensionData"] {
        assert_eq!(after.get(key), before.get(key));
    }
}
#[test]
fn links_expand_selection_move_trim_slip_and_report_bypass_offset() {
    let mut s = paired();
    assert_eq!(
        linked_selection(s.root().unwrap(), &ids(&["b"]), false).unwrap(),
        ids(&["b", "sound"])
    );
    assert_eq!(
        linked_selection(s.root().unwrap(), &ids(&["b"]), true).unwrap(),
        ids(&["b"])
    );
    roundtrip(
        &mut s,
        Edit::Move {
            layer_ids: ids(&["b", "sound"]),
            delta: 1.,
            linked: true,
        },
    );
    assert_eq!(n(&layer(&s, "sound"), &["from"]), 5.);
    assert_eq!(
        sync_badges(s.root().unwrap(), "b").unwrap()[0].offset_seconds,
        0.
    );
    run(
        &mut s,
        Edit::Move {
            layer_ids: ids(&["b"]),
            delta: 0.5,
            linked: false,
        },
    );
    assert_eq!(
        sync_badges(s.root().unwrap(), "b").unwrap()[0].offset_seconds,
        0.5
    );
    roundtrip(
        &mut s,
        Edit::Trim {
            layer_ids: ids(&["b"]),
            edge: Edge::In,
            delta: 0.5,
            linked: true,
        },
    );
    assert_eq!(
        sync_badges(s.root().unwrap(), "b").unwrap()[0].offset_seconds,
        0.5
    );
    roundtrip(
        &mut s,
        Edit::Slip {
            layer_ids: ids(&["b"]),
            delta: 0.5,
            linked: true,
        },
    );
    assert_eq!(
        sync_badges(s.root().unwrap(), "b").unwrap()[0].offset_seconds,
        0.5
    );
}
#[test]
fn link_unlink_undo_relink_and_invalid_pair_rejection() {
    let mut s = paired();
    reject(
        &mut s,
        Link::Link {
            video_id: "a".into(),
            audio_id: "sound".into(),
            link_id: "pair2".into(),
        },
    );
    roundtrip(
        &mut s,
        Link::Unlink {
            layer_ids: ids(&["b"]),
        },
    );
    assert_eq!(
        linked_selection(s.root().unwrap(), &ids(&["b"]), false).unwrap(),
        ids(&["b"])
    );
    roundtrip(
        &mut s,
        Link::Link {
            video_id: "b".into(),
            audio_id: "sound".into(),
            link_id: "again".into(),
        },
    );
    reject(
        &mut s,
        Link::Link {
            video_id: "a".into(),
            audio_id: "c".into(),
            link_id: "bad".into(),
        },
    );
}
#[test]
fn linked_delete_and_bypass_delete_never_leave_dangling_pair() {
    for linked in [true, false] {
        let mut s = paired();
        roundtrip(
            &mut s,
            Edit::Delete {
                layer_ids: ids(&["b"]),
                linked,
            },
        );
        assert!(pm_kernel::model::find_layer(s.root().unwrap(), "b").is_none());
        if !linked {
            assert!(layer(&s, "sound").get("editLink").is_none());
        }
    }
}
#[test]
fn split_linked_pair_preserves_continuity_and_has_distinct_tail_link() {
    let mut s = paired();
    roundtrip(
        &mut s,
        Edit::Split {
            layer_id: "b".into(),
            time: 6.,
            tails: vec![
                SplitTail {
                    layer_id: "b".into(),
                    tail_id: "bt".into(),
                },
                SplitTail {
                    layer_id: "sound".into(),
                    tail_id: "st".into(),
                },
            ],
            linked: true,
            tail_link_id: Some("tail-pair".into()),
        },
    );
    assert_eq!(n(&layer(&s, "bt"), &["from"]), 6.);
    assert_eq!(n(&layer(&s, "bt"), &["dur"]), 2.);
    assert_eq!(n(&layer(&s, "bt"), &["d", "trim", "v"]), 8.);
    assert_eq!(
        n(&layer(&s, "bt"), &["p", "position.x", "kf", "0", "t"]),
        -1.
    );
    assert_eq!(
        linked_selection(s.root().unwrap(), &ids(&["bt"]), false).unwrap(),
        ids(&["bt", "st"])
    );
    assert_eq!(
        sync_badges(s.root().unwrap(), "bt").unwrap()[0].offset_seconds,
        0.
    );
}
#[test]
fn split_requires_complete_fresh_ids_and_minimum_frames() {
    for (time, tail) in [(4., "bt"), (6., "a"), (7.99, "bt")] {
        let mut s = abc();
        reject(
            &mut s,
            Edit::Split {
                layer_id: "b".into(),
                time,
                tails: vec![SplitTail {
                    layer_id: "b".into(),
                    tail_id: tail.into(),
                }],
                linked: true,
                tail_link_id: None,
            },
        );
    }
    let mut s = paired();
    reject(
        &mut s,
        Edit::Split {
            layer_id: "b".into(),
            time: 6.,
            tails: vec![SplitTail {
                layer_id: "b".into(),
                tail_id: "bt".into(),
            }],
            linked: true,
            tail_link_id: Some("t".into()),
        },
    );
}
#[test]
fn ripple_does_not_silently_widen_linked_track_scope() {
    let mut s = paired();
    reject(
        &mut s,
        Edit::RippleDelete {
            layer_ids: ids(&["a"]),
            tracks: tracks(),
            linked: true,
        },
    );
    roundtrip(
        &mut s,
        Edit::RippleDelete {
            layer_ids: ids(&["a"]),
            tracks: vec![TrackTarget::new(0), TrackTarget::new(1)],
            linked: true,
        },
    );
    assert_eq!(n(&layer(&s, "b"), &["from"]), 0.);
    assert_eq!(n(&layer(&s, "sound"), &["from"]), 0.);
}
#[test]
fn cancel_and_multi_update_drag_is_one_undo() {
    let mut s = abc();
    let before = root(&s);
    let g = s.begin_gesture("Roll", Origin::Interface).unwrap();
    s.update_gesture_from_start(g, &[Command::from(roll(0.5))])
        .unwrap();
    s.update_gesture_from_start(g, &[Command::from(roll(1.))])
        .unwrap();
    s.cancel_gesture(g).unwrap();
    assert_eq!(root(&s), before);
    let g = s.begin_gesture("Roll", Origin::Interface).unwrap();
    s.update_gesture_from_start(g, &[roll(0.5).into()]).unwrap();
    s.update_gesture_from_start(g, &[roll(1.).into()]).unwrap();
    s.commit_gesture(g).unwrap();
    assert_eq!(n(&layer(&s, "a"), &["dur"]), 5.);
    s.undo().unwrap();
    assert_eq!(root(&s), before);
    s.redo().unwrap();
    assert_eq!(n(&layer(&s, "a"), &["dur"]), 5.);
}
#[test]
fn generated_origin_respects_hand_edits() {
    let mut b = clip("b", 4., 4., 0);
    b["locked_intent"] = json!({"timing":true});
    let mut s = store(vec![b]);
    let before = root(&s);
    assert!(
        s.execute(
            Edit::Move {
                layer_ids: ids(&["b"]),
                delta: 1.,
                linked: true
            }
            .into(),
            Origin::Agent
        )
        .is_err()
    );
    assert_eq!(root(&s), before);
}
#[test]
fn nested_composition_and_track_group_ids_are_resolved_per_command() {
    let mut a = clip("inner-a", 0., 4., 2);
    a["group"] = json!("inner-g");
    let mut b = clip("inner-b", 4., 4., 2);
    b["group"] = json!("inner-g");
    let p = json!({"id":"P","name":"nested","compId":"C1","assets":{"src":{"kind":"video","dur":100}},"comps":{"C1":{"id":"C1","fps":30,"dur":40,"w":100,"h":100,"layers":[clip("outer",0.,4.,0)]},"C2":{"id":"C2","fps":24,"dur":40,"w":100,"h":100,"layers":[json!({"id":"inner-g","type":"group","from":0,"dur":40}),a,b]}}});
    let mut s =
        pm_kernel::DocumentStore::new_untitled(pm_model::json::parse(&p.to_string()).unwrap());
    let outer = layer(&s, "outer");
    roundtrip(
        &mut s,
        Edit::RippleDelete {
            layer_ids: ids(&["inner-a"]),
            tracks: vec![TrackTarget {
                group_id: Some("inner-g".into()),
                track: 2,
            }],
            linked: true,
        },
    );
    assert_eq!(n(&layer(&s, "inner-b"), &["from"]), 0.);
    assert_eq!(layer(&s, "outer"), outer);
}

#[test]
fn roll_and_slide_update_all_linked_track_pairs() {
    for is_slide in [false, true] {
        let mut s = store(vec![
            clip("a", 0., 4., 0),
            clip("b", 4., 4., 0),
            clip("c", 8., 4., 0),
            audio("aa", 0., 4., 1),
            audio("ba", 4., 4., 1),
            audio("ca", 8., 4., 1),
        ]);
        for (v, a, l) in [("a", "aa", "pa"), ("b", "ba", "pb"), ("c", "ca", "pc")] {
            run(
                &mut s,
                Link::Link {
                    video_id: v.into(),
                    audio_id: a.into(),
                    link_id: l.into(),
                },
            );
        }
        roundtrip(&mut s, if is_slide { slide(1.) } else { roll(1.) });
        for (v, a) in [("a", "aa"), ("b", "ba"), ("c", "ca")] {
            for key in ["from", "dur"] {
                assert_eq!(layer(&s, v).get(key), layer(&s, a).get(key));
            }
            assert_eq!(
                sync_badges(s.root().unwrap(), v).unwrap()[0].offset_seconds,
                0.
            );
        }
    }
}
#[test]
fn linked_ripple_trim_requires_matching_bounds_and_propagates_once() {
    let mut s = paired();
    let tracks = vec![TrackTarget::new(0), TrackTarget::new(1)];
    roundtrip(
        &mut s,
        Edit::RippleTrim {
            layer_id: "b".into(),
            edge: Edge::Out,
            delta: -1.,
            tracks: tracks.clone(),
            linked: true,
        },
    );
    assert_eq!(n(&layer(&s, "sound"), &["dur"]), 3.);
    assert_eq!(n(&layer(&s, "c"), &["from"]), 7.);
    run(
        &mut s,
        Edit::Trim {
            layer_ids: ids(&["sound"]),
            edge: Edge::Out,
            delta: -0.5,
            linked: false,
        },
    );
    reject(
        &mut s,
        Edit::RippleTrim {
            layer_id: "b".into(),
            edge: Edge::Out,
            delta: -1.,
            tracks,
            linked: true,
        },
    );
}
#[test]
fn failed_late_command_and_failed_peer_edit_leave_transaction_unchanged() {
    let mut s = paired();
    let before = root(&s);
    let revision = s.revision();
    let commands = vec![
        Edit::Move {
            layer_ids: ids(&["b"]),
            delta: 1.,
            linked: true,
        }
        .into(),
        roll(50.).into(),
    ];
    assert!(
        s.transact("Atomic edits", Origin::Agent, &commands)
            .is_err()
    );
    assert_eq!(root(&s), before);
    assert_eq!(s.revision(), revision);
    let mut a = audio("sound", 4., 4., 1);
    a["d"]["trim"]["v"] = json!(96.);
    let mut s = store(vec![clip("b", 4., 4., 0), a]);
    link(&mut s);
    reject(
        &mut s,
        Edit::Slip {
            layer_ids: ids(&["b"]),
            delta: 1.,
            linked: true,
        },
    );
}
#[test]
fn group_null_matches_absent_group_and_unrelated_tracks_stay_put() {
    let a = clip("a", 0., 4., 0);
    let mut b = clip("b", 4., 4., 0);
    b["group"] = json!(null);
    let mut s = store(vec![a, b, clip("untargeted", 4., 4., 1)]);
    let before = layer(&s, "untargeted");
    roundtrip(&mut s, roll(0.5));
    assert_eq!(layer(&s, "untargeted"), before);
}
#[test]
fn malformed_source_content_or_links_are_refused_without_panicking() {
    let mut b = clip("b", 4., 4., 0);
    b.as_object_mut().unwrap().remove("d");
    let mut s = store(vec![b]);
    reject(
        &mut s,
        Edit::Move {
            layer_ids: ids(&["b"]),
            delta: 1.,
            linked: true,
        },
    );
    let mut b = clip("b", 4., 4., 0);
    b["editLink"] = json!({"id":"orphan","phase":0});
    let mut s = store(vec![b]);
    assert!(linked_selection(s.root().unwrap(), &ids(&["b"]), false).is_err());
    reject(
        &mut s,
        Edit::Move {
            layer_ids: ids(&["b"]),
            delta: 1.,
            linked: true,
        },
    );
}
