mod common;
#[path = "pro_fixture.rs"]
mod fixture;
use fixture::*;
use pm_kernel::commands::pro_edit::TrackTarget;
use pm_kernel::commands::pro_links::{ProLinkCommand, linked_selection, sync_badges};
use pm_kernel::commands::pro_source::*;
use serde_json::json;
fn command(mode: SourceEditMode, start: f64, end: f64) -> ProSourceCommand {
    ProSourceCommand {
        comp_id: None,
        asset_id: "src".into(),
        points: EditPoints {
            source_in: Some(10.),
            source_out: Some(10. + end - start),
            timeline_in: Some(start),
            ..Default::default()
        },
        mode,
        targets: vec![SourceTarget {
            media: SourceMedia::Video,
            track: TrackTarget::new(0),
            layer_id: "new".into(),
        }],
        link_id: None,
        splits: vec![],
    }
}
fn split(c: &mut ProSourceCommand, id: &str, tail: &str) {
    c.splits.push(SourceSplit {
        layer_id: id.into(),
        tail_id: tail.into(),
        tail_link_id: None,
    });
}
fn has(s: &pm_kernel::DocumentStore, id: &str) -> bool {
    pm_kernel::model::find_layer(s.root().unwrap(), id).is_some()
}
#[test]
fn resolve_each_missing_point_and_consistent_four_points() {
    let p = [Some(2.), Some(5.), Some(8.), Some(11.)];
    for missing in 0..5 {
        let mut p = p;
        if missing < 4 {
            p[missing] = None;
        }
        let r = EditPoints {
            source_in: p[0],
            source_out: p[1],
            timeline_in: p[2],
            timeline_out: p[3],
            playhead: 0.,
        }
        .resolve()
        .unwrap();
        assert_eq!(
            r,
            ResolvedEdit {
                source_in: 2.,
                source_out: 5.,
                timeline_in: 8.,
                timeline_out: 11.
            }
        );
    }
}
#[test]
fn resolve_playhead_and_reject_ambiguous_inconsistent_or_nonfinite_marks() {
    let r = EditPoints {
        source_in: Some(2.),
        source_out: Some(5.),
        playhead: 7.,
        ..Default::default()
    }
    .resolve()
    .unwrap();
    assert_eq!(r.timeline_in, 7.);
    for p in [
        EditPoints::default(),
        EditPoints {
            source_in: Some(5.),
            source_out: Some(2.),
            playhead: 0.,
            ..Default::default()
        },
        EditPoints {
            source_in: Some(2.),
            source_out: Some(5.),
            timeline_in: Some(8.),
            timeline_out: Some(12.),
            ..Default::default()
        },
        EditPoints {
            source_in: None,
            source_out: Some(1.),
            timeline_in: Some(0.),
            timeline_out: Some(2.),
            ..Default::default()
        },
        EditPoints {
            source_in: Some(f64::NAN),
            source_out: Some(5.),
            timeline_in: Some(0.),
            ..Default::default()
        },
    ] {
        assert!(p.resolve().is_err());
    }
}
#[test]
fn insert_at_cut_ripples_only_target_track() {
    let mut s = store(vec![
        clip("a", 0., 4., 0),
        clip("b", 4., 4., 0),
        clip("c", 8., 4., 0),
        clip("untargeted", 4., 4., 2),
    ]);
    let u = layer(&s, "untargeted");
    roundtrip(&mut s, command(SourceEditMode::Insert, 4., 6.));
    assert_eq!(n(&layer(&s, "b"), &["from"]), 6.);
    assert_eq!(n(&layer(&s, "c"), &["from"]), 10.);
    assert_eq!(n(&layer(&s, "new"), &["d", "trim", "v"]), 10.);
    assert_eq!(layer(&s, "untargeted"), u);
}
#[test]
fn insert_inside_clip_splits_animation_and_keeps_every_source_frame() {
    let mut s = abc();
    let mut c = command(SourceEditMode::Insert, 6., 8.);
    split(&mut c, "b", "bt");
    roundtrip(&mut s, c);
    assert_eq!(n(&layer(&s, "b"), &["dur"]), 2.);
    let t = layer(&s, "bt");
    assert_eq!(n(&t, &["from"]), 8.);
    assert_eq!(n(&t, &["dur"]), 2.);
    assert_eq!(n(&t, &["d", "trim", "v"]), 8.);
    assert_eq!(n(&t, &["p", "position.x", "kf", "0", "t"]), -1.);
    assert!(t.get("extensionData").is_some());
    assert_eq!(n(&layer(&s, "c"), &["from"]), 10.);
}
#[test]
fn overwrite_whole_clip_does_not_ripple_later_clips() {
    let mut s = abc();
    let before = layer(&s, "c");
    roundtrip(&mut s, command(SourceEditMode::Overwrite, 4., 8.));
    assert!(!has(&s, "b"));
    assert_eq!(layer(&s, "c"), before);
}
#[test]
fn overwrite_middle_retains_both_fragments_at_matching_source_times() {
    let mut s = abc();
    let mut c = command(SourceEditMode::Overwrite, 5., 7.);
    split(&mut c, "b", "bt");
    roundtrip(&mut s, c);
    assert_eq!(n(&layer(&s, "b"), &["dur"]), 1.);
    let t = layer(&s, "bt");
    assert_eq!(n(&t, &["from"]), 7.);
    assert_eq!(n(&t, &["dur"]), 1.);
    assert_eq!(n(&t, &["d", "trim", "v"]), 9.);
    assert_eq!(n(&t, &["p", "position.x", "kf", "0", "t"]), -2.);
    assert_eq!(n(&layer(&s, "c"), &["from"]), 8.);
}
#[test]
fn overwrite_partial_edges_retains_original_ids() {
    let mut s = abc();
    roundtrip(&mut s, command(SourceEditMode::Overwrite, 3., 5.));
    assert_eq!(n(&layer(&s, "a"), &["dur"]), 3.);
    assert_eq!(n(&layer(&s, "b"), &["from"]), 5.);
    assert_eq!(n(&layer(&s, "b"), &["d", "trim", "v"]), 7.);
    assert_eq!(n(&layer(&s, "b"), &["dur"]), 3.);
}
#[test]
fn missing_asset_handles_min_frame_and_composition_end_refuse() {
    for mut c in [
        command(SourceEditMode::Insert, 39., 41.),
        command(SourceEditMode::Overwrite, 1., 1.001),
    ] {
        let mut s = abc();
        reject(&mut s, c.clone());
        c.asset_id = "missing".into();
        reject(&mut s, c);
    }
    let mut c = command(SourceEditMode::Insert, 4., 6.);
    c.points.source_in = Some(99.);
    c.points.source_out = Some(101.);
    reject(&mut abc(), c);
    let mut s = store(vec![clip("last", 36., 4., 0)]);
    reject(&mut s, command(SourceEditMode::Insert, 0., 2.));
}
#[test]
fn locked_overwrite_target_and_downstream_insert_refuse_without_partial_change() {
    for mode in [SourceEditMode::Overwrite, SourceEditMode::Insert] {
        let mut b = clip("b", 4., 4., 0);
        b["lock"] = json!(true);
        let mut s = store(vec![clip("a", 0., 4., 0), b]);
        reject(&mut s, command(mode, 4., 6.));
    }
}
#[test]
fn split_ids_are_exact_fresh_and_not_accepted_when_unused() {
    let mut c = command(SourceEditMode::Overwrite, 5., 7.);
    reject(&mut abc(), c.clone());
    split(&mut c, "b", "a");
    reject(&mut abc(), c);
    let mut c = command(SourceEditMode::Overwrite, 4., 8.);
    split(&mut c, "b", "unused");
    reject(&mut abc(), c);
}
#[test]
fn av_insert_creates_persistent_pair_and_stream_only_targets_are_explicit() {
    let mut s = abc();
    let mut c = command(SourceEditMode::Overwrite, 4., 8.);
    c.targets.push(SourceTarget {
        media: SourceMedia::Audio,
        track: TrackTarget::new(1),
        layer_id: "new-audio".into(),
    });
    c.link_id = Some("inserted-pair".into());
    roundtrip(&mut s, c);
    assert_eq!(
        linked_selection(s.root().unwrap(), &ids(&["new"]), false).unwrap(),
        ids(&["new", "new-audio"])
    );
    assert_eq!(
        sync_badges(s.root().unwrap(), "new").unwrap()[0].offset_seconds,
        0.
    );
    assert_eq!(
        layer(&s, "new")
            .get("d")
            .unwrap()
            .get("embeddedAudio")
            .unwrap()
            .as_bool(),
        Some(false)
    );
    let mut c = command(SourceEditMode::Overwrite, 0., 2.);
    c.targets[0].media = SourceMedia::Audio;
    let mut s = abc();
    roundtrip(&mut s, c);
    assert_eq!(
        layer(&s, "new").get("type").unwrap().as_str(),
        Some("audio")
    );
}
#[test]
fn target_both_existing_linked_tracks_and_give_matching_tail_link_ids() {
    let mut s = store(vec![clip("b", 4., 4., 0), audio("sound", 4., 4., 1)]);
    run(
        &mut s,
        ProLinkCommand::Link {
            video_id: "b".into(),
            audio_id: "sound".into(),
            link_id: "original".into(),
        },
    );
    let mut c = command(SourceEditMode::Overwrite, 5., 7.);
    split(&mut c, "b", "bt");
    c.splits[0].tail_link_id = Some("tails".into());
    reject(&mut s, c.clone());
    c.targets.push(SourceTarget {
        media: SourceMedia::Audio,
        track: TrackTarget::new(1),
        layer_id: "new-audio".into(),
    });
    c.link_id = Some("new-pair".into());
    c.splits.push(SourceSplit {
        layer_id: "sound".into(),
        tail_id: "st".into(),
        tail_link_id: Some("tails".into()),
    });
    roundtrip(&mut s, c);
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
fn source_edit_rejects_duplicate_tracks_or_unsupported_stream() {
    let mut c = command(SourceEditMode::Insert, 4., 6.);
    c.targets.push(SourceTarget {
        media: SourceMedia::Audio,
        track: TrackTarget::new(0),
        layer_id: "audio".into(),
    });
    c.link_id = Some("p".into());
    reject(&mut abc(), c);
    let p = json!({"id":"P","compId":"C1","fps":30,"dur":40,"w":100,"h":100,"assets":{"src":{"kind":"video","dur":100,"hasAudio":false}},"layers":[]});
    let mut s =
        pm_kernel::DocumentStore::new_untitled(pm_model::json::parse(&p.to_string()).unwrap());
    let mut c = command(SourceEditMode::Insert, 4., 6.);
    c.targets[0].media = SourceMedia::Audio;
    reject(&mut s, c);
}
#[test]
fn links_source_data_and_undo_survive_pmv_save_reopen() {
    let dir = common::TempDir::new("pro-save");
    let mut s = store(vec![clip("b", 4., 4., 0), audio("sound", 4., 4., 1)]);
    run(
        &mut s,
        ProLinkCommand::Link {
            video_id: "b".into(),
            audio_id: "sound".into(),
            link_id: "saved".into(),
        },
    );
    let expected = root(&s);
    common::save_as(&mut s, &dir.path("pro.pmv")).unwrap();
    let mut reopened = common::open_store(&dir.path("pro.pmv"));
    assert_eq!(root(&reopened), expected);
    assert_eq!(
        linked_selection(reopened.root().unwrap(), &ids(&["b"]), false).unwrap(),
        ids(&["b", "sound"])
    );
    reopened.undo().unwrap();
    assert!(layer(&reopened, "b").get("editLink").is_none());
    reopened.redo().unwrap();
    assert_eq!(root(&reopened), expected);
}

#[test]
fn group_headers_and_unchanged_nonmedia_strips_do_not_block_source_edits() {
    let title = json!({"id":"title","type":"text","from":0.,"dur":2.,"d":{"text":"keep"},"p":{}});
    let group = json!({"id":"group","type":"group","from":0.,"dur":40.});
    let mut s = store(vec![title, group, clip("b", 4., 4., 0)]);
    let title = layer(&s, "title");
    let group = layer(&s, "group");
    roundtrip(&mut s, command(SourceEditMode::Insert, 4., 6.));
    assert_eq!(layer(&s, "title"), title);
    assert_eq!(layer(&s, "group"), group);
}
#[test]
fn inserted_video_uses_real_layer_defaults_and_imported_dimensions() {
    let p = json!({"id":"P","compId":"C1","fps":30,"dur":40,"w":200,"h":100,"assets":{"src":{"kind":"video","dur":100,"hasAudio":true,"w":640,"h":480}},"layers":[]});
    let mut s =
        pm_kernel::DocumentStore::new_untitled(pm_model::json::parse(&p.to_string()).unwrap());
    run(&mut s, command(SourceEditMode::Insert, 0., 2.));
    let c = layer(&s, "new");
    assert_eq!(n(&c, &["p", "position.x", "v"]), 100.);
    assert_eq!(n(&c, &["p", "opacity", "v"]), 100.);
    assert_eq!(n(&c, &["p", "scale.x", "v"]), 100.);
    assert_eq!(n(&c, &["d", "w"]), 640.);
    assert_eq!(n(&c, &["d", "h"]), 480.);
}
