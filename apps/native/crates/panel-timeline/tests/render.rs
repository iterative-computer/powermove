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
fn selected_clips_keep_their_hue_with_a_white_rim_and_exact_authored_times() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        let t = Tokens::new(scheme, Accessibility::default());
        let f = scene(&t).prepare(viewport(&t), &t).unwrap();
        let clip = f.ops.iter().find(|o| o.id == "clip:selected").unwrap();
        assert_eq!(clip.detail["from"], 0.125);
        assert_eq!(clip.detail["duration"], 5.375);
        // A strip that starts before the view runs on past its edge and is
        // cut straight there (no rounded end where it does not end).
        let column = t.metrics.layer_column as f64;
        assert!(clip.bounds.x < column - clip.radius);
        assert_eq!(clip.clip.unwrap().x, column);
        // The body is the layer's strip in its selected state: its own hue
        // with a bright rim inside the edge, never an accent fill or ring.
        let material = t.strip(t.colors.labels.teal, pm_design::StripState::Selected);
        let rest = t.strip(t.colors.labels.teal, pm_design::StripState::Rest);
        assert_eq!(clip.shape, "clip");
        assert_eq!(clip.detail["top"], material.top);
        assert_eq!(clip.detail["ring"], material.ring.0);
        assert!(material.ring.1 > rest.ring.1 * 2., "selection is the rim");
        // Accessibility reads selection from the strip, with no ring to find.
        let node = f.clip_nodes().into_iter().find(|n| n.id == "selected");
        assert!(node.is_some_and(|n| n.selected));
        assert!(
            f.ops
                .iter()
                .all(|o| o.color != t.colors.accent || !o.id.starts_with("clip"))
        );
        let content = f
            .ops
            .iter()
            .find(|o| o.id == "clip-content:selected")
            .unwrap();
        // The name starts clear of the trim grip inside the strip's edge.
        let grip = t.trim_grip();
        // The name stays in sight at the view's edge.
        assert_eq!(
            content.bounds.x,
            column + (grip.inset + grip.width + t.space.xs) as f64
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
    // Needs render shows in the warning colour; stale frames don't show.
    let needs = f.ops.iter().find(|o| o.id == "render:0").unwrap();
    assert_eq!(needs.color, t.colors.warning);
    assert!(!f.ops.iter().any(|o| o.id == "render:4"));
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
    // The strip is drawn inside its times; its trim zone still reaches the
    // authored end and past it, at the strip's height only.
    assert_eq!(f.hit_clip(end, y), Some(("selected", Some(Edge::Out))));
    assert_eq!(f.hit_clip(end + 4., y), Some(("selected", Some(Edge::Out))));
    assert_eq!(f.hit_clip(end + 6., y), None);
    // v1's reach inside the strip: five points in from its drawn end trims,
    // with room for an end drawn on a half point (the strip stops
    // `strip_gap / 2` short of the layer's out point).
    let drawn = end - t.strip_gap() as f64 / 2.;
    for inside in [3., 5., 5.5] {
        assert_eq!(f.hit_clip(drawn - inside, y), Some(("selected", Some(Edge::Out))));
    }
    assert_eq!(f.hit_clip(drawn - 6.5, y), Some(("selected", None)));
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
    let t = tokens();
    let v = viewport(&t);
    let g = graph(true, false);
    let area = Rect::new(v.layer_column, 220., v.width - v.layer_column, 220.);
    let legend = Rect::new(v.layer_column, 220., v.width - v.layer_column, 28.);
    let ops = g.prepare(v, area, legend, &t).unwrap();
    assert_eq!(
        ops.iter().find(|o| o.id == "curve:a:position.x").unwrap().color,
        t.graph_curve("position.x", 0)
    );
    let legend = ops.iter().find(|o| o.id == "graph-legend").unwrap();
    assert_eq!(legend.detail["items"][0]["text"], "X Position");
    // Two selected keys: v1's transform box with its eight grips.
    assert!(ops.iter().any(|o| o.id == "graph-box"));
    assert_eq!(
        ops.iter()
            .filter(|o| o.id.starts_with("graph-transform-handle"))
            .count(),
        8
    );
    assert!(
        ops.iter()
            .filter(|o| o.id != "graph-legend" && o.id != "graph-legend-ground")
            .all(|o| o.clip.is_some())
    );
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

/// One X Position curve from 1 s (0) to 3 s (1), eased out of its first key,
/// both keys selected or not; in the Speed Graph or the Value Graph.
fn graph(selected: bool, speed: bool) -> pm_panel_timeline::graph_render::Graph {
    use pm_panel_timeline::graph_render::*;
    let a = Point {
        seconds: 1.,
        value: 0.,
    };
    let b = Point {
        seconds: 3.,
        value: 1.,
    };
    let key = |id: &str, point: Point, handles| GraphPoint {
        id: id.into(),
        point,
        selected,
        shape: KeyShape::Bezier,
        handles,
        active_handle: None,
    };
    let mut g = Graph::new(
        vec![Curve {
            id: "a:position.x".into(),
            path: "position.x".into(),
            name: "X Position".into(),
            index: 0,
            layers: 1,
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
            points: vec![
                key(
                    "k1",
                    a,
                    vec![(
                        EaseSide::Out,
                        Point {
                            seconds: 1.5,
                            value: 0.,
                        },
                    )],
                ),
                key("k2", b, vec![]),
            ],
        }],
        -0.22,
        1.22,
    )
    .unwrap();
    g.speed = speed;
    g
}

#[test]
fn the_transform_box_frames_two_selected_keys_and_scales_as_v1() {
    use pm_panel_timeline::graph_render::*;
    let t = tokens();
    let v = viewport(&t);
    let area = Rect::new(v.layer_column, 220., v.width - v.layer_column, 220.);
    assert!(graph(false, false).transform_box(v, area).is_none());
    let b = graph(true, false).transform_box(v, area).unwrap();
    // It stands 8 points off the keys.
    let left = v.layer_column + (1. - v.scroll_time) * v.pps;
    assert!((b.rect.x - (left - BOX_INSET)).abs() < 1e-9);
    assert_eq!(b.keys, ["k1", "k2"]);
    assert_eq!((b.time, b.value), ((1., 3.), (0., 1.)));
    // Each grip is taken within its reach, the nearest; nothing else is.
    for grip in Grip::ALL {
        let [x, y] = grip.at(b.rect);
        assert_eq!(b.grip_at(x + 4., y), Some(grip));
    }
    let [x, y] = Grip::E.at(b.rect);
    assert_eq!(b.grip_at(x + GRIP_REACH + 1., y), None);
    // The east edge scales time about the west edge; values stay.
    assert_eq!(b.scale(Grip::E, 5., 9., false), (1., 0.5, 2., 1.));
    // The north-west corner scales both about the south-east corner (the
    // north grips carry the highest value).
    assert_eq!(b.scale(Grip::NW, 2., 0.5, false), (3., 0., 0.5, 0.5));
    // Cmd: about the centre.
    assert_eq!(b.scale(Grip::E, 4., 0., true), (2., 0.5, 2., 1.));
    // The Speed Graph frames the keys but offers no grips.
    let speed = graph(true, true).transform_box(v, area).unwrap();
    let [x, y] = Grip::E.at(speed.rect);
    assert_eq!(speed.grip_at(x, y), None);
}

#[test]
fn the_playhead_marks_each_curve_with_its_value_as_v1() {
    use pm_panel_timeline::graph_render::*;
    let t = tokens();
    let v = viewport(&t);
    let area = Rect::new(v.layer_column, 220., v.width - v.layer_column, 220.);
    let g = graph(false, false);
    let curve = &g.curves()[0];
    // Held before and after the keys; on the eased segment between.
    assert_eq!(curve.value_at(0.), Some(0.));
    assert_eq!(curve.value_at(9.), Some(1.));
    let mid = curve.value_at(2.).unwrap();
    assert!((mid - 0.5).abs() < 1e-6, "{mid}");
    let early = curve.value_at(1.25).unwrap();
    assert!(early > 0. && early < 0.25, "{early}");
    // A head on the plot: a dot on the curve in its ink, the value beside it.
    let time = v.scroll_time + 200. / v.pps;
    let marks = g.playhead_marks(v, area, time, &t);
    assert_eq!(marks.len(), 1);
    let m = &marks[0];
    assert_eq!(m.color, t.graph_curve("position.x", 0));
    assert_eq!(m.text, value_text(curve.value_at(time).unwrap()));
    assert!((m.label.unwrap() - (m.y - 7.).clamp(area.y + 10., area.y + area.height - 3.)).abs() < 1e-9);
    // None within 35 points of the layer column or 40 of the right edge.
    let at = |x: f64| v.scroll_time + (x - v.layer_column) / v.pps;
    assert!(g.playhead_marks(v, area, at(v.layer_column + 20.), &t).is_empty());
    assert!(g.playhead_marks(v, area, at(v.width - 20.), &t).is_empty());
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
    assert_eq!(
        wave.color,
        t.strip(t.colors.labels.green, pm_design::StripState::Rest)
            .primary
    );
    let bars = wave.detail["bars"].as_array().unwrap();
    let inset = 2. * t.metrics.border as f64;
    // The waveform fills the strip's part in view.
    let clip = body.clip.unwrap();
    let (x0, x1) = (
        body.bounds.x.max(clip.x),
        (body.bounds.x + body.bounds.width).min(clip.x + clip.width),
    );
    assert_eq!(bars.len(), (x1 - x0 - 2. * inset).floor() as usize);
    assert_eq!(wave.bounds.x, x0 + inset);
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
        // A hidden layer's strip is the material's disabled state.
        t.strip(t.colors.labels.teal, pm_design::StripState::Disabled)
            .opacity as f64
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
/// A collapsed layer's summary marks stay inside its strip (v1's closed
/// clip): keys before its in point or after its out point are not drawn, the
/// keys at its ends stand just inside them, a dense run is thinned to one
/// mark per glyph width (the rest kept for hit-testing, not painted), and
/// every mark is clipped to the strip so nothing hangs under or beside it.
#[test]
fn summary_marks_are_drawn_inside_their_strip_and_clipped_to_it() {
    // Both appearances, at the least, the default and the most row height a
    // workspace may set (22, 30 and 48 points).
    for (scheme, height) in [Scheme::Light, Scheme::Dark]
        .into_iter()
        .flat_map(|s| [22., 30., 48.].map(|h| (s, h)))
    {
        let mut t = Tokens::new(scheme, Accessibility::default());
        t.apply_patch([("metrics.track_row", height)]).unwrap();
        let mut short = clip("short");
        short.from = 2.;
        short.duration = 1.;
        // Before the in point, on it, a dense run inside, on the out point,
        // and after it.
        let mut keys = vec![key("before", 1.2), key("in", 2.)];
        keys.extend((0..40).map(|i| key(&format!("dense{i}"), 2.3 + i as f64 * 0.005)));
        keys.extend([key("out", 3.), key("after", 4.5)]);
        let s = Scene::new(
            vec![row(
                "short",
                RowContent::Layer {
                    clip: Some(short),
                    summary: keys.into(),
                },
            )],
            12.,
            30.,
            &t,
        )
        .unwrap();
        let f = s.prepare(viewport(&t), &t).unwrap();
        let strip = f.ops.iter().find(|o| o.id == "clip:short").unwrap().bounds;
        let marks: Vec<_> = f.ops.iter().filter(|o| o.id.starts_with("key:")).collect();
        let named = |name: &str| marks.iter().any(|o| o.id.contains(&format!("\"{name}\"")));
        assert!(
            !named("before") && !named("after"),
            "outside the in and out points"
        );
        assert!(named("in") && named("out") && named("dense0"));
        for o in &marks {
            let b = o.bounds;
            // Inside the strip, clear of its rounded ends.
            assert!(
                b.x >= strip.x && b.x + b.width <= strip.x + strip.width,
                "{}",
                o.id
            );
            assert!(
                b.y >= strip.y && b.y + b.height <= strip.y + strip.height,
                "{}",
                o.id
            );
            let clip = o.clip.unwrap();
            assert!(
                clip.x >= strip.x
                    && clip.y >= strip.y
                    && clip.x + clip.width <= strip.x + strip.width
                    && clip.y + clip.height <= strip.y + strip.height,
                "{} is clipped to its strip",
                o.id
            );
        }
        // The dense run (0.2 s at 100 pt/s = 20 pt) paints a mark at most
        // every glyph width.
        let painted: Vec<f64> = marks
            .iter()
            .filter(|o| !o.covered && o.id.contains("dense"))
            .map(|o| o.bounds.x)
            .collect();
        assert!(painted.len() >= 2 && painted.len() < 6, "{painted:?}");
        assert!(
            painted
                .windows(2)
                .all(|w| w[1] - w[0] >= marks[0].bounds.width - 1e-9)
        );
    }
}

/// A layer a tenth of a second long, zoomed out to a few points a second (a
/// Text-tool layer made with the playhead past the composition's end lasts
/// that long), still has a strip a person can see, select and trim: v1 drew
/// every clip at least 4 px wide.
#[test]
fn a_very_short_layer_keeps_a_visible_strip_at_any_zoom() {
    let t = tokens();
    let mut short = clip("short");
    short.from = 3.;
    short.duration = 0.1;
    for pps in [4., 10., 100.] {
        let v = Viewport { pps, ..viewport(&t) };
        let f = scene_with(&t, short.clone()).prepare(v, &t).unwrap();
        let strip = f.ops.iter().find(|o| o.id == "clip:short").expect("a strip");
        assert!(strip.bounds.width >= 4., "{pps}: {}", strip.bounds.width);
        let start = v.layer_column + (3. - v.scroll_time) * pps;
        assert!((strip.bounds.x - (start + t.strip_gap() as f64 / 2.)).abs() < 1e-9);
    }
}

/// Five points in from either end of a strip trims that end, wherever the
/// zoom puts its ends between points (acceptance found the out end giving
/// only about two of v1's five).
#[test]
fn both_ends_of_a_strip_trim_five_points_in() {
    let t = tokens();
    let mut c = clip("mid");
    c.from = 3.;
    c.duration = 1.;
    for pps in [34.5, 35., 35.25, 100.] {
        let v = Viewport { pps, ..viewport(&t) };
        let f = scene_with(&t, c.clone()).prepare(v, &t).unwrap();
        let o = f.ops.iter().find(|o| o.id == "clip:mid").expect("a strip");
        let (left, right) = (o.bounds.x, o.bounds.x + o.bounds.width);
        let y = o.bounds.y + o.bounds.height / 2.;
        for inside in [1., 3., 5., 5.5] {
            assert_eq!(f.hit_clip(left + inside, y), Some(("mid", Some(Edge::In))), "{pps} in {inside}");
            assert_eq!(f.hit_clip(right - inside, y), Some(("mid", Some(Edge::Out))), "{pps} out {inside}");
        }
        if right - left > 14. {
            assert_eq!(f.hit_clip(left + 6.5, y), Some(("mid", None)), "{pps}");
            assert_eq!(f.hit_clip(right - 6.5, y), Some(("mid", None)), "{pps}");
        }
    }
}
