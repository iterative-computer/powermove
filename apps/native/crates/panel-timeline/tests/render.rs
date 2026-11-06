#[path = "support/render_fixture.rs"]
mod fixture;
use fixture::*;
use pm_design::{Accessibility, Scheme, Tokens};
use pm_kernel::commands::keyframes::EaseSide;
use pm_panel_timeline::render::*;
use serde_json::json;
use std::collections::HashMap;
fn tokens() -> Tokens {
    Tokens::new(Scheme::Dark, Accessibility::default())
}
#[test]
fn all_reviewed_paint_states_have_wire_snapshots_in_each_appearance() {
    let mut snapshots = vec![];
    for scheme in [Scheme::Light, Scheme::Dark] {
        for contrast in [false, true] {
            for motion in [false, true] {
                let t = Tokens::new(
                    scheme,
                    Accessibility {
                        increase_contrast: contrast,
                        reduce_motion: motion,
                        ..Default::default()
                    },
                );
                let f = scene(&t).prepare(viewport(&t), &t).unwrap();
                let mut recorder = Recorder::default();
                f.paint(&mut recorder);
                snapshots.push(json!({"scheme":format!("{scheme:?}"),"contrast":contrast,"reduceMotion":motion,"frame":f.wire(),"playhead":f.playhead(3.12345,&t).unwrap().iter().map(Op::wire).collect::<Vec<_>>()}));
                assert_eq!(recorder.frames[0]["ops"], f.wire()["ops"]);
            }
        }
    }
    let wire = serde_json::to_string_pretty(&snapshots).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/render.json");
    if std::env::var_os("PM_UPDATE_TIMELINE_RENDER").is_some() {
        std::fs::write(&path, &wire).unwrap();
    }
    // Workspace feature unification can enable serde_json preserve_order.
    // JSON object order is immaterial; ordered paint arrays remain exact.
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&wire).unwrap();
    assert_eq!(actual, expected);
}
#[test]
fn selected_clips_use_label_mix_and_text_ring_and_exact_authored_times() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        let t = Tokens::new(scheme, Accessibility::default());
        let f = scene(&t).prepare(viewport(&t), &t).unwrap();
        let clip = f.ops.iter().find(|o| o.id == "clip:selected").unwrap();
        assert_eq!(clip.detail["from"], 0.125);
        assert_eq!(clip.detail["duration"], 5.375);
        assert_eq!(clip.bounds.x, t.metrics.layer_column as f64); // sticky visible edge
        assert_ne!(clip.color, t.colors.accent);
        assert_ne!(clip.color, t.colors.labels.teal);
        let ring = f.ops.iter().find(|o| o.id == "clip-ring:selected").unwrap();
        assert_eq!(ring.color, t.colors.text);
        assert_eq!(ring.stroke, t.metrics.playhead as f64);
        let content = f
            .ops
            .iter()
            .find(|o| o.id == "clip-content:selected")
            .unwrap();
        assert_eq!(
            content.bounds.x,
            clip.bounds.x + (t.metrics.inset - t.metrics.inner_radius) as f64
        );
    }
}
#[test]
fn ruler_scissors_work_area_render_states_and_three_diamonds_are_explicit() {
    let t = tokens();
    let f = scene(&t).prepare(viewport(&t), &t).unwrap();
    for (id, shape) in [
        ("static", "diamond"),
        ("animated", "diamond.lefthalf.filled"),
        ("at-playhead", "diamond.fill"),
    ] {
        assert_eq!(
            f.ops
                .iter()
                .find(|o| o.id == format!("toggle:{id}"))
                .unwrap()
                .shape,
            shape
        );
    }
    assert!(
        f.ops
            .iter()
            .filter(|o| o.id.starts_with("clip:"))
            .all(|o| o.clip.unwrap().y == t.metrics.ruler as f64)
    );
    assert!(f.ops.iter().any(|o| o.shape == "stripes-45"));
    assert!(f.ops.iter().any(|o| o.id == "kept:3" && o.stroke == 0.));
    assert!(
        !f.ops
            .iter()
            .any(|o| o.id == "render:0" || o.id == "render:4")
    );
    let mut reduced = t.clone();
    reduced.accessibility.reduce_motion = true;
    let f = scene(&reduced)
        .prepare(viewport(&reduced), &reduced)
        .unwrap();
    assert!(!f.ops.iter().any(|o| o.shape == "stripes-45"));
    assert_eq!(
        f.ops.iter().find(|o| o.id == "render:1").unwrap().opacity,
        0.5
    );
}
#[test]
fn trim_hit_tests_use_authored_edges_and_ignore_gutter_ruler_and_offscreen_edges() {
    let t = tokens();
    let f = scene(&t).prepare(viewport(&t), &t).unwrap();
    let y = t.metrics.ruler as f64 + 10.;
    assert_eq!(
        f.hit_clip(t.metrics.layer_column as f64 + 1., y),
        Some(("selected", None))
    );
    let end = t.metrics.layer_column as f64 + (5.5 - 1.) * 100.;
    assert_eq!(f.hit_clip(end - 1., y), Some(("selected", Some(Edge::Out))));
    assert_eq!(f.hit_clip(t.metrics.layer_column as f64 - 1., y), None);
    assert_eq!(f.hit_clip(end, 3.), None);
    assert_eq!(
        viewport(&t).seconds(t.metrics.layer_column as f64 - 1.),
        None
    );
}
#[test]
fn sixty_thousand_rows_have_bounded_paint_ax_and_id_navigation() {
    let t = tokens();
    let s = large_scene(60_000, &t);
    let mut v = viewport(&t);
    v.scroll_y = 30_000. * t.metrics.track_row as f64 + 3.;
    let f = s.prepare(v, &t).unwrap();
    assert!(f.range.len() < 50);
    assert!(f.ops.len() < 400);
    assert_eq!(f.total, 60_000);
    let ids: HashMap<_, _> = f
        .accessibility
        .iter()
        .enumerate()
        .map(|(i, row)| {
            (
                row["id"].as_str().unwrap().to_string(),
                accesskit::NodeId(i as u64 + 1),
            )
        })
        .collect();
    let ax = f
        .accessibility_tree(accesskit::NodeId(0), &ids, Some("layer-30000"))
        .unwrap();
    let root = &ax.nodes.last().unwrap().1;
    assert_eq!(root.row_count(), Some(60_000));
    assert_eq!(root.children().len(), f.range.len());
    assert!(root.active_descendant().is_some());
    assert_eq!(s.navigate("layer-30000", 1).unwrap().id, "layer-30001");
    assert_eq!(s.navigate("layer-0", -1).map(|r| &r.id), None);
    let (id, offset) = s.anchor(v).unwrap();
    assert_eq!(id, "layer-30000");
    assert_eq!(offset, 3.);
    assert_eq!(s.reanchor(id, offset), Some(v.scroll_y));
    for tick in 0..120 {
        assert_eq!(f.playhead(1. + tick as f64 / 120., &t).unwrap().len(), 2);
    }
}
#[test]
fn malformed_geometry_and_nan_never_reach_native_painter() {
    let t = tokens();
    let s = scene(&t);
    let mut v = viewport(&t);
    v.pps = f64::NAN;
    assert_eq!(s.prepare(v, &t).unwrap_err(), Error::InvalidGeometry);
    let f = s.prepare(viewport(&t), &t).unwrap();
    assert_eq!(
        f.playhead(f64::INFINITY, &t).unwrap_err(),
        Error::InvalidTime
    );
    assert!(f.playhead(-100., &t).unwrap().is_empty());
}

#[test]
fn graph_curves_handles_selection_and_names_use_neutral_spec_marks() {
    use pm_panel_timeline::graph_render::*;
    let t = tokens();
    let v = viewport(&t);
    let a = Point {
        seconds: 1.,
        value: 0.,
    };
    let b = Point {
        seconds: 3.,
        value: 1.,
    };
    let mut g = Graph::new(
        vec![Curve {
            id: "x".into(),
            name: "X".into(),
            focused: true,
            segments: vec![Segment {
                from: a,
                to: b,
                controls: Some([
                    Point {
                        seconds: 1.5,
                        value: 0.,
                    },
                    Point {
                        seconds: 2.5,
                        value: 1.,
                    },
                ]),
            }],
            points: vec![GraphPoint {
                id: "key".into(),
                point: a,
                selected: true,
                handles: vec![(
                    EaseSide::Out,
                    Point {
                        seconds: 1.5,
                        value: 0.,
                    },
                )],
                active_handle: Some(EaseSide::Out),
            }],
        }],
        0.,
        1.,
    )
    .unwrap();
    g.grid = vec![(0., "0".into()), (1., "1".into())];
    g.selection = Some(Rect::new(300., 250., 100., 80.));
    let ops = g
        .prepare(
            v,
            Rect::new(v.layer_column, 220., v.width - v.layer_column, 220.),
            &t,
        )
        .unwrap();
    assert_eq!(
        ops.iter().find(|o| o.id == "curve:x").unwrap().color,
        t.colors.text
    );
    assert_eq!(
        ops.iter()
            .find(|o| o.id == "curve-name:x")
            .unwrap()
            .text
            .as_deref(),
        Some("X")
    );
    assert_eq!(
        ops.iter()
            .filter(|o| o.id.starts_with("graph-transform-handle"))
            .count(),
        4
    );
    assert!(ops.iter().all(|o| o.clip.is_some()));
    let wire =
        serde_json::to_string_pretty(&ops.iter().map(Op::wire).collect::<Vec<_>>()).unwrap() + "\n";
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graph-render.json");
    if std::env::var_os("PM_UPDATE_TIMELINE_RENDER").is_some() {
        std::fs::write(&path, &wire).unwrap();
    }
    // Workspace feature unification can enable serde_json preserve_order.
    // JSON object order is immaterial; ordered paint arrays remain exact.
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let actual: serde_json::Value = serde_json::from_str(&wire).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn transport_repaints_visible_diamonds_from_shared_key_indexes() {
    let t = tokens();
    let f = scene(&t).prepare(viewport(&t), &t).unwrap();
    let before = f.wire();
    let at = f.transport(1.1, &t).unwrap();
    assert_eq!(
        at.iter().find(|o| o.id == "toggle:animated").unwrap().shape,
        "diamond.fill"
    );
    let between = f.transport(1.2, &t).unwrap();
    assert_eq!(
        between
            .iter()
            .find(|o| o.id == "toggle:animated")
            .unwrap()
            .shape,
        "diamond.lefthalf.filled"
    );
    assert_eq!(
        between
            .iter()
            .find(|o| o.id == "toggle:static")
            .unwrap()
            .shape,
        "diamond"
    );
    assert_eq!(f.wire(), before);
}

#[test]
fn sixty_thousand_clips_in_one_track_cull_by_time() {
    let t = tokens();
    let clips = (0..60_000)
        .map(|i| {
            let mut c = clip(&format!("clip-{i}"));
            c.from = i as f64 * 2.;
            c.duration = 1.;
            c
        })
        .collect();
    let s = Scene::new(
        vec![row("V1", RowContent::Track { clips })],
        120_000.,
        30.,
        &t,
    )
    .unwrap();
    let mut v = viewport(&t);
    v.scroll_time = 60_000.;
    let f = s.prepare(v, &t).unwrap();
    let clips: Vec<_> = f
        .ops
        .iter()
        .filter(|op| op.id.starts_with("clip:"))
        .collect();
    assert!(clips.len() <= 4);
    assert_eq!(clips[0].id, "clip:clip-30000");
}

#[test]
fn every_paint_operation_has_a_unique_id_even_for_summary_and_property_keys() {
    let t = tokens();
    let f = scene(&t).prepare(viewport(&t), &t).unwrap();
    let ids = f
        .ops
        .iter()
        .map(|op| op.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), f.ops.len());
}
