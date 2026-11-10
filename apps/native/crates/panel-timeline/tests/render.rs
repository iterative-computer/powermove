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
fn selected_clips_keep_their_material_with_an_inside_accent_ring_and_exact_authored_times() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        let t = Tokens::new(scheme, Accessibility::default());
        let f = scene(&t).prepare(viewport(&t), &t).unwrap();
        let clip = f.ops.iter().find(|o| o.id == "clip:selected").unwrap();
        assert_eq!(clip.detail["from"], 0.125);
        assert_eq!(clip.detail["duration"], 5.375);
        assert_eq!(clip.bounds.x, t.metrics.layer_column as f64); // sticky visible edge
        // The body is the label's tactile material, never an accent fill.
        let material = t.clip_material(t.colors.labels.teal);
        assert_eq!(clip.shape, "clip");
        assert_eq!(clip.detail["top"], material.top);
        assert_eq!(clip.detail["seam"], material.seam);
        // The ring sits inside the clip's edge, so it never grows into a neighbour.
        let ring = f.ops.iter().find(|o| o.id == "clip-ring:selected").unwrap();
        assert_eq!((ring.shape, ring.bounds), ("ring", clip.bounds));
        assert_eq!(ring.color, t.colors.accent);
        assert_eq!(ring.stroke, 2. * t.metrics.border as f64);
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
    // The render bar stops its stripes; an offline clip's hatch is static.
    assert!(
        !f.ops
            .iter()
            .any(|o| o.id.starts_with("render:") && o.shape == "stripes-45")
    );
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
fn a_marquee_covers_rows_the_frame_did_not_build_above_and_below_the_panel() {
    let t = tokens();
    let rows = (0..40)
        .map(|i| {
            row(
                &format!("V{i}"),
                RowContent::Track {
                    clips: vec![clip(&format!("c{i}"))],
                },
            )
        })
        .collect();
    let s = Scene::new(rows, 10., 30., &t).unwrap();
    let mut v = viewport(&t);
    v.scroll_y = s.span(15).unwrap().1;
    let top = |i: usize| t.metrics.ruler as f64 + s.span(i).unwrap().1 - v.scroll_y;
    let built = s.prepare(v, &t).unwrap().range;
    assert!(built.start > 5 && built.end < 35, "{built:?}");
    // From inside row 5 (scrolled above the panel) down into row 34 (below it).
    let (from, to) = (top(5) + 2., top(35) - 2.);
    let area = Rect::new(v.layer_column + 10., from, 40., to - from);
    let covered = s.marquee(v, &t, area);
    let expected: Vec<_> = (5..35).map(|i| format!("c{i}")).collect();
    assert_eq!(covered.clips, expected);
    assert!(covered.keys.is_empty());
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
#[test]
fn track_ground_is_the_themed_panel_material() {
    for reduce_transparency in [false, true] {
        let mut t = Tokens::new(
            Scheme::Dark,
            Accessibility {
                reduce_transparency,
                ..Default::default()
            },
        );
        // A theme's `--bg-panel` arrives as a `colors.panel` patch.
        t.apply_patch([("colors.panel", 0x203040 as f64)]).unwrap();
        let f = scene(&t).prepare(viewport(&t), &t).unwrap();
        let ground = f.ops.iter().find(|o| o.id == "ground").unwrap();
        assert_eq!(ground.color, 0x203040);
        let alpha = if reduce_transparency {
            1.
        } else {
            t.translucency.panel as f64
        };
        assert_eq!(ground.opacity, alpha);
    }
}
#[test]
fn offline_clips_carry_the_media_offline_sign() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        let t = Tokens::new(scheme, Accessibility::default());
        let f = scene(&t).prepare(viewport(&t), &t).unwrap();
        let fill = f.ops.iter().find(|o| o.id == "clip:offline").unwrap();
        assert_eq!(fill.color, t.colors.viewer);
        let hatch = f.ops.iter().find(|o| o.id == "clip-hatch:offline").unwrap();
        assert_eq!((hatch.shape, hatch.color), ("stripes-45", t.colors.danger));
        assert_eq!(hatch.opacity, t.translucency.offline_hatch as f64);
        let lead = f
            .ops
            .iter()
            .find(|o| o.id == "clip-offline:offline")
            .unwrap();
        assert_eq!(lead.text.as_deref(), Some("Offline"));
        // The name follows the lead, in `text`.
        let name = f
            .ops
            .iter()
            .find(|o| o.id == "clip-content:offline")
            .unwrap();
        assert!(name.bounds.x > lead.bounds.x && name.color == t.colors.text);
        assert!(!f.ops.iter().any(|o| o.id == "clip-hatch:selected"));
    }
}

#[test]
fn a_missing_effect_badge_keeps_its_room_and_explains_itself() {
    let t = Tokens::new(Scheme::Dark, Accessibility::default());
    let f = scene(&t).prepare(viewport(&t), &t).unwrap();
    // The fixture has no missing effects: no badge, no tooltip.
    assert!(
        !f.ops
            .iter()
            .any(|o| o.id.starts_with("clip-missing-effect:"))
    );
    let mut clip = clip("long");
    clip.name = "A very long clip name that runs the whole width of its bar".into();
    clip.missing_effect = Some("Deep Glow".into());
    let s = scene_with(&t, clip);
    let f = s.prepare(viewport(&t), &t).unwrap();
    let badge = f
        .ops
        .iter()
        .find(|o| o.id == "clip-missing-effect:long")
        .unwrap();
    let name = f.ops.iter().find(|o| o.id == "clip-content:long").unwrap();
    // The name ends before the badge starts: nothing overlaps.
    assert!(name.bounds.x + name.bounds.width <= badge.bounds.x + 1e-6);
    assert_eq!(name.detail["truncate"], "tail");
    let (x, y) = (
        badge.bounds.x + badge.bounds.width / 2.,
        badge.bounds.y + badge.bounds.height / 2.,
    );
    let tip = f.tooltip_at(x, y).unwrap();
    assert!(tip.contains("Deep Glow"), "{tip}");
    assert_eq!(f.tooltip_at(name.bounds.x + 1., y), None);
}
#[test]
fn audio_clips_draw_one_bar_per_point_from_their_trimmed_peaks() {
    let t = tokens();
    let v = viewport(&t);
    let mut audio = clip("a");
    audio.label = Label::Green;
    // 10 buckets per second; a two-second trim skips the first 20.
    let peaks: Vec<f32> = (0..100).map(|i| if i < 20 { 1. } else { 0.25 }).collect();
    audio.audio = Some(Some(Waveform {
        peaks: peaks.into(),
        rate: 10.,
        trim: 2.,
    }));
    let f = scene_with(&t, audio.clone()).prepare(v, &t).unwrap();
    let body = f.ops.iter().find(|o| o.id == "clip:a").unwrap();
    let wave = f.ops.iter().find(|o| o.id == "clip-content:a").unwrap();
    assert_eq!(wave.shape, "waveform");
    assert_eq!(wave.color, t.clip_material(t.colors.labels.green).primary);
    let bars = wave.detail["bars"].as_array().unwrap();
    let inset = 2. * t.metrics.border as f64;
    assert_eq!(
        bars.len(),
        (body.bounds.width - 2. * inset).floor() as usize
    );
    assert_eq!(wave.bounds.x, body.bounds.x + inset);
    assert!(
        bars.iter().all(|b| b.as_f64() == Some(0.25)),
        "trimmed peaks only"
    );
    // Still reading: no bars (the painter draws the middle hairline).
    audio.audio = Some(None);
    let f = scene_with(&t, audio.clone()).prepare(v, &t).unwrap();
    let wave = f.ops.iter().find(|o| o.id == "clip-content:a").unwrap();
    assert_eq!(
        (wave.shape, wave.detail["bars"].as_array().map(Vec::len)),
        ("waveform", Some(0))
    );
    // Offline audio names itself with the Offline lead instead.
    audio.media = Media::Offline;
    let f = scene_with(&t, audio).prepare(v, &t).unwrap();
    let name = f.ops.iter().find(|o| o.id == "clip-content:a").unwrap();
    assert_eq!(name.shape, "text");
    assert!(f.ops.iter().any(|o| o.id == "clip-offline:a"));
}
#[test]
fn hidden_layers_draw_at_half_strength_and_keys_mark_their_span_ends() {
    let t = tokens();
    let mut c = clip("h");
    c.hidden = true;
    let f = scene_with(&t, c).prepare(viewport(&t), &t).unwrap();
    assert_eq!(
        f.ops.iter().find(|o| o.id == "clip:h").unwrap().opacity,
        0.5
    );
    let f = scene(&t).prepare(viewport(&t), &t).unwrap();
    let half: Vec<_> = f
        .ops
        .iter()
        .filter(|o| o.id.starts_with("key:") && !o.detail["half"].is_null())
        .map(|o| o.detail["half"].as_str().unwrap())
        .collect();
    // Each property's first key (linear) opens its span; its last is a hold
    // square, which keeps its own shape.
    assert!(
        !half.is_empty() && half.iter().all(|h| *h == "right"),
        "{half:?}"
    );
    assert!(
        f.ops.iter().any(|o| o.shape == "square"),
        "hold keys are squares"
    );
}
