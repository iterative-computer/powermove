use super::*;
use pm_design::{Accessibility as A11y, Scheme, Tokens};
use pm_kernel::{Call, Caller, HistoryCall, KernelActor, KernelConfig, OpenSource, Reply, Request};
use pm_model::{Value, json::parse};
use std::collections::BTreeSet;

fn tokens() -> Tokens {
    Tokens::new(Scheme::Dark, A11y::default())
}
fn vertex(i: usize) -> path::Vertex {
    path::Vertex {
        id: format!("v{i}"),
        position: path::Point {
            x: i as f64,
            y: -(i as f64),
        },
        incoming: path::Point { x: -4., y: 2. },
        outgoing: path::Point { x: 4., y: -2. },
    }
}
fn path_state() -> path::PathState {
    path::PathState {
        id: "path1".into(),
        vertices: (0..5).map(vertex).collect(),
        controls_visible: true,
        ..Default::default()
    }
}
fn picker() -> colour::Picker {
    colour::Picker {
        fill: None,
        space: colour::Space::Srgb,
        notation: colour::Notation::Hex,
        hue_mapping: colour::HueMapping::Closest,
        outside_project_gamut: false,
        project_space: colour::Space::Srgb,
        previous: "#112233".into(),
        current: "#445566".into(),
        recent: vec![],
    }
}
const PROJECT: &str = r##"{"id":"P","name":"Synthetic editing fixture","w":200,"h":100,"fps":30,"dur":10,"bg":"#000000","assets":{},"comps":{},"layers":[
{"id":"T","type":"text","name":"Headline","from":0,"dur":10,"on":true,"p":{},"d":{"text":"Original 🎬","size":64,"boxWidth":0,"boxHeight":0,"animators":[{"id":"ta1","name":"Fade","enabled":false,"p":{"opacity":{"v":0,"kf":[{"i":"k1","t":0,"v":0}]}},"opaque":{"extensionValue":42}}]}},
{"id":"S","type":"shape","name":"Shape","from":0,"dur":10,"on":true,"p":{},"d":{"paths":[]}},
{"id":"L","type":"text","name":"Locked","lock":true,"from":0,"dur":10,"on":true,"p":{},"d":{"text":"Keep"}}]}"##;
fn actor() -> (KernelActor, kernel::EditingKernel) {
    let actor = KernelActor::spawn(KernelConfig::default());
    let handle = actor.handle();
    handle
        .open("P", OpenSource::Untitled(parse(PROJECT).unwrap()))
        .wait()
        .unwrap()
        .unwrap();
    (actor, kernel::EditingKernel { handle })
}
fn target(k: &kernel::EditingKernel, layer: &str) -> kernel::Target {
    kernel::Target::from_published(&k.handle.published("P").unwrap(), layer).unwrap()
}
fn layer(k: &kernel::EditingKernel, id: &str) -> Value {
    let p = k.handle.published("P").unwrap();
    let root = p.snapshot.root().unwrap();
    let loc = pm_kernel::model::find_layer(root, id).unwrap();
    pm_kernel::model::layer_at(root, &loc).unwrap().to_value()
}
fn submit(k: &kernel::EditingKernel, t: &kernel::Target, r: Request) -> pm_kernel::Ack {
    {
        let ack = k.submit(t, r).unwrap().wait().unwrap().unwrap();
        assert!(!matches!(ack.reply, Reply::Throw(_)), "{ack:?}");
        ack
    }
}

#[test]
fn path_handles_use_dark_tokens_in_every_appearance_and_accessibility_state() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        for contrast in [false, true] {
            for transparency in [false, true] {
                for motion in [false, true] {
                    let t = Tokens::new(
                        scheme,
                        A11y {
                            increase_contrast: contrast,
                            reduce_transparency: transparency,
                            reduce_motion: motion,
                        },
                    );
                    let dark = Tokens::new(Scheme::Dark, t.accessibility);
                    let p = path_state().plan(&t);
                    assert_eq!(p.scheme, Scheme::Dark);
                    assert_eq!(p.stroke, dark.colors.accent);
                    assert_eq!(p.stroke_width, if contrast { 2. } else { 1. });
                    assert_eq!(p.vertices[0].side, t.metrics.keyframe - 2.);
                    assert_eq!(p.tangent_diameter, t.metrics.keyframe - 3.);
                }
            }
        }
    }
}
#[test]
fn path_selection_hover_and_neighbours_are_distinct() {
    let mut state = path_state();
    state.selected.insert("v2".into());
    state.hovered = Some("v1".into());
    let p = state.plan(&tokens());
    assert_eq!(
        p.vertices
            .iter()
            .map(|v| v.show_tangents)
            .collect::<Vec<_>>(),
        vec![false, true, true, true, false]
    );
    assert_eq!(p.vertices[1].side, 9.);
    assert_eq!(p.vertices[2].fill, tokens().colors.accent);
    assert_eq!(p.vertices[2].stroke, tokens().colors.selected_text);
    assert_eq!(p.vertices[2].ax.selected, Some(true));
    assert_eq!(p.vertices[0].ax.label, "Point 1 of 5, 0, -0 pixels");
    assert_eq!(p.vertices[2].ax.label, "Point 3 of 5, 2, -2 pixels");
    assert_eq!(p.vertices[0].ax.role, "AXButton");
}
#[test]
fn closed_path_wraps_neighbours_and_ids_survive_reordering() {
    let mut state = path_state();
    state.closed = true;
    state.selected.insert("v0".into());
    let before = state.plan(&tokens());
    assert!(before.vertices[4].show_tangents);
    state.vertices.reverse();
    let after = state.plan(&tokens());
    assert_eq!(before.vertices[0].ax.id, after.vertices[4].ax.id);
}
#[test]
fn drawing_has_first_ring_and_dashed_rubber_band() {
    let mut s = path_state();
    s.drawing = true;
    s.can_close = true;
    s.pointer = Some(path::Point { x: 50., y: 60. });
    let p = s.plan(&tokens());
    assert_eq!(p.vertices[0].close_ring, Some(tokens().metrics.keyframe));
    assert!(p.vertices[1].close_ring.is_none());
    assert_eq!(
        p.rubber_band,
        Some((vertex(4).position, s.pointer.unwrap()))
    );
    assert_eq!(p.rubber_band_dash, [4., 3.]);
    s.closed = true;
    assert!(s.plan(&tokens()).rubber_band.is_none());
}
#[test]
fn locked_and_hidden_paths_never_expose_handles() {
    let mut s = path_state();
    s.locked = true;
    let p = s.plan(&tokens());
    assert!(p.vertices.is_empty());
    assert_eq!(p.banner, Some("Layer is locked"));
    s.locked = false;
    s.controls_visible = false;
    assert!(s.plan(&tokens()).vertices.is_empty());
}
#[test]
fn mask_label_changes_only_the_path_stroke() {
    let mut s = path_state();
    s.mask_label_colour = Some(tokens().colors.labels.green);
    let p = s.plan(&tokens());
    assert_eq!(p.stroke, tokens().colors.labels.green);
    assert_eq!(p.vertices[0].stroke, tokens().colors.accent);
}
#[test]
fn drag_is_from_start_and_option_breaks_mirroring() {
    let original = vertex(2);
    let d = path::Point { x: 7., y: 9. };
    let moved = path::drag(&original, path::Handle::Vertex, d, false);
    assert_eq!(moved.position, original.position.plus(d));
    assert_eq!(moved.incoming, original.incoming);
    for handle in [
        path::Handle::Incoming,
        path::Handle::Outgoing,
        path::Handle::New,
    ] {
        let mirrored = path::drag(&original, handle, d, false);
        assert_eq!(mirrored.incoming.x, -mirrored.outgoing.x);
        assert_eq!(mirrored.incoming.y, -mirrored.outgoing.y);
        let broken = path::drag(&original, handle, d, true);
        if handle == path::Handle::Incoming {
            assert_eq!(broken.outgoing, original.outgoing);
        } else {
            assert_eq!(broken.incoming, original.incoming);
        }
    }
    let option = path::drag(&original, path::Handle::Vertex, d, true);
    assert_eq!(option.position, original.position);
    assert_eq!(option.outgoing, original.outgoing.plus(d));
}
#[test]
fn pen_close_and_delete_precedence_matches_the_tag() {
    let hit = || path::Hit {
        index: 0,
        count: 3,
        active: true,
        point: true,
        clicks: 1,
        option: false,
        command: false,
    };
    assert_eq!(path::hit_action(hit()), path::HitAction::Close);
    assert_eq!(
        path::hit_action(path::Hit { count: 2, ..hit() }),
        path::HitAction::Drag
    );
    assert_eq!(
        path::hit_action(path::Hit {
            command: true,
            ..hit()
        }),
        path::HitAction::Delete
    );
    assert_eq!(
        path::hit_action(path::Hit {
            clicks: 2,
            command: true,
            ..hit()
        }),
        path::HitAction::Close
    );
    assert_eq!(
        path::hit_action(path::Hit {
            option: true,
            ..hit()
        }),
        path::HitAction::Drag
    );
}
#[test]
fn system_cursors_have_exact_template_symbols() {
    use path::Cursor::*;
    assert_eq!(
        [Select, Draw, Add, Delete, Close, Convert].map(path::Cursor::symbol),
        [
            None,
            Some("pencil.tip"),
            Some("pencil.tip.crop.circle.badge.plus"),
            Some("pencil.tip.crop.circle.badge.minus"),
            Some("pencil.tip.crop.circle.badge.arrow.forward"),
            Some("arrow.triangle.branch")
        ]
    );
}
#[test]
fn text_overlay_is_system_input_with_dark_selection_and_exact_overset_copy() {
    let p = path::text_overlay(
        "T",
        "Headline",
        true,
        &Tokens::new(Scheme::Light, A11y::default()),
    );
    assert_eq!(p.editor.role, "AXTextArea");
    assert_eq!(p.editor.label, "Headline");
    assert!(p.system_caret && p.system_marked_text && p.dashed_bounds);
    assert_eq!(p.selection_colour, tokens().colors.accent);
    assert_eq!(p.selection_alpha, 0.3);
    assert_eq!(
        p.overset,
        Some((9., tokens().colors.warning, "More text than fits"))
    );
    assert!(
        path::text_overlay("T", "Headline", false, &tokens())
            .overset
            .is_none()
    );
}
#[test]
fn every_field_kind_has_the_specified_native_text_policy() {
    use text::{FieldKind::*, Setting::*};
    for field in [Number, Timecode, Notation] {
        let p = field.policy();
        assert_eq!(
            (
                p.spelling,
                p.grammar,
                p.substitutions,
                p.smart_quotes,
                p.writing_tools
            ),
            (Off, Off, Off, Off, Off)
        );
    }
    for field in [Expression, Code] {
        let p = field.policy();
        assert_eq!(
            (p.spelling, p.substitutions, p.smart_quotes),
            (Off, Off, Off)
        );
        assert_eq!(p.writing_tools, SystemDefault);
    }
    for field in [Canvas, AgentComposer, LayerName, Notes] {
        assert_eq!(field.policy(), text::TextPolicy::default());
    }
}
#[test]
fn focus_returns_to_its_original_panel_and_ime_keeps_escape_local() {
    let mut f = text::Focus::default();
    assert!(f.editor_shortcuts_allowed());
    f.enter("name", "inspector");
    f.enter("size", "other-field");
    assert!(!f.editor_shortcuts_allowed());
    f.set_marked_text(true);
    assert_eq!(f.escape(), None);
    assert!(!f.editor_shortcuts_allowed());
    f.set_marked_text(false);
    assert_eq!(f.escape(), Some("inspector".into()));
    assert!(f.editor_shortcuts_allowed());
    assert_eq!(f.escape(), None);
}
#[test]
fn point_and_paragraph_creation_leave_auto_height_to_the_kernel() {
    assert_eq!(
        text::TextCreation::from_gesture(None),
        Some(text::TextCreation::Point)
    );
    assert_eq!(
        text::TextCreation::from_gesture(Some(42.)),
        Some(text::TextCreation::Paragraph { width: 42. })
    );
    for v in [f64::NAN, f64::INFINITY, -1., 0.] {
        assert!(text::TextCreation::from_gesture(Some(v)).is_none());
    }
}
#[test]
fn all_canvas_exit_routes_commit_once_after_marked_text_finishes() {
    for reason in [
        text::TextExit::Escape,
        text::TextExit::CommandReturn,
        text::TextExit::OutsideClick,
    ] {
        let mut session = text::CanvasText::new("T", "Before");
        session.composing(true);
        session.replace("日本語 🎬".into());
        assert_eq!(session.finish(reason), None);
        session.composing(false);
        assert_eq!(session.finish(reason), Some("日本語 🎬"));
        assert_eq!(session.finish(reason), None);
        assert!(!session.replace("Lost".into()));
    }
}
#[test]
fn animator_empty_and_style_selection_copy_is_exact() {
    let mut a = animators::Animators {
        layer: "T".into(),
        ..Default::default()
    };
    assert_eq!(
        a.empty_copy(),
        Some("Add an animator to move, fade or style characters over time.")
    );
    assert_eq!(
        [
            animators::HEADING,
            animators::ADD,
            animators::ADD_PROPERTY,
            animators::STYLE,
            animators::SELECT_FIRST
        ],
        [
            "Text Animators",
            "Add Text Animator",
            "Add Animator Property",
            "Style Selected Text",
            "Select text on the canvas first"
        ]
    );
    for selection in [None, Some(("other".into(), 0, 2)), Some(("T".into(), 1, 1))] {
        a.text_selection = selection;
        assert!(!a.can_style());
    }
    a.text_selection = Some(("T".into(), 0, 2));
    assert!(a.can_style());
    a.locked = true;
    assert!(!a.can_style());
}
#[test]
fn animator_disabled_content_stays_editable_and_collapsed_animation_is_visible() {
    let a = animators::Animator::from_value(
        &parse(r#"{"id":"a","name":"Fade","enabled":false,"p":{"opacity":{"v":0,"kf":[{}]}}}"#)
            .unwrap(),
    )
    .unwrap();
    let mut state = animators::Animators {
        items: vec![a],
        layer: "T".into(),
        ..Default::default()
    };
    let p = state.plans(&tokens()).remove(0);
    assert!(p.expanded && p.rows_editable);
    assert_eq!(p.text_colour, tokens().colors.muted);
    assert_eq!(p.ax.label, "Fade, animator 1 of 1");
    assert_eq!(p.ax.role, "AXGroup");
    assert_eq!(p.enabled_label, "Fade enabled");
    assert!(state.toggle("a"));
    assert!(!state.toggle("missing"));
    let p = state.plans(&tokens()).remove(0);
    assert!(!p.expanded && p.half_diamond);
    state.locked = true;
    assert!(!state.plans(&tokens())[0].rows_editable);
}
#[test]
fn animator_menu_copy_order_boundaries_and_danger_are_exact() {
    let a = animators::Animator::from_value(&parse(r#"{"id":"a","name":"A"}"#).unwrap()).unwrap();
    let mut s = animators::Animators {
        items: vec![
            a.clone(),
            animators::Animator {
                id: "b".into(),
                ..a
            },
        ],
        ..Default::default()
    };
    let p = s.plans(&tokens());
    assert_eq!(
        p[0].menu.iter().map(|v| v.0).collect::<Vec<_>>(),
        vec!["Rename…", "Duplicate", "Move Up", "Move Down", "Remove"]
    );
    assert!(!p[0].menu[2].1);
    assert!(p[0].menu[3].1);
    assert!(!p[1].menu[3].1);
    assert!(p[1].menu[2].1);
    assert!(p[0].menu[4].2);
    assert_eq!(p[0].remove_colour, tokens().colors.danger);
    s.locked = true;
    assert!(s.plans(&tokens())[0].menu.iter().all(|v| !v.1));
}
#[test]
fn animator_selector_rows_and_property_menu_preserve_unknown_data() {
    let source =
        parse(r#"{"id":"a","name":"Fade","extra":{"foreign":42},"p":{"opacity":{"v":0}}}"#)
            .unwrap();
    let state = animators::Animators {
        items: vec![animators::Animator::from_value(&source).unwrap()],
        ..Default::default()
    };
    assert_eq!(state.items[0].source, source);
    assert_eq!(
        state.available_properties("a", &[("opacity", "Opacity"), ("x", "Position X")]),
        vec![("x", "Position X")]
    );
    assert_eq!(
        animators::SELECTOR_ROWS,
        [
            "Start",
            "End",
            "Offset",
            "Unit",
            "Shape",
            "Softness",
            "Direction",
            "Order",
            "Stagger",
            "Mode"
        ]
    );
    let p = state.plans(&tokens());
    assert_eq!(
        (p[0].selector_heading, p[0].properties_heading),
        ("Range", "Properties")
    );
}
#[test]
fn all_fill_types_and_colour_property_have_correct_picker_visibility() {
    let mut s = picker();
    let t = tokens();
    assert!(s.plan(&t, true).fill_segments.is_empty());
    for fill in colour::FillType::ALL {
        s.fill = Some(fill);
        let p = s.plan(&t, true);
        assert_eq!(p.fill_segments, ["None", "Solid", "Linear", "Radial"]);
        assert_eq!(p.picker_body_visible, fill != colour::FillType::None);
        assert_eq!(
            p.gradient,
            matches!(fill, colour::FillType::Linear | colour::FillType::Radial)
        );
        assert_eq!(p.linear_angle, fill == colour::FillType::Linear);
    }
}
#[test]
fn colour_settings_sections_checkmarks_and_copy_labels_are_exact() {
    for space in colour::Space::ALL {
        for notation in colour::Notation::ALL {
            for hue in colour::HueMapping::ALL {
                let mut s = picker();
                s.space = space;
                s.notation = notation;
                s.hue_mapping = hue;
                let p = s.plan(&tokens(), true);
                for section in ["Colour Space", "Notation", "Hue Mapping"] {
                    assert_eq!(
                        p.menu
                            .iter()
                            .filter(|m| m.section == section && m.checked)
                            .count(),
                        1
                    );
                }
                assert!(
                    p.menu
                        .iter()
                        .any(|m| m.label == format!("Copy as {}", notation.label()))
                );
            }
        }
    }
    assert_eq!(
        colour::Notation::ALL.map(colour::Notation::label),
        ["Hex", "RGB", "HSL", "OKLCH", "OKLab", "P3"]
    );
    assert_eq!(
        colour::HueMapping::ALL.map(colour::HueMapping::label),
        [
            "Find Closest Color",
            "Maintain Chroma",
            "Maintain Lightness"
        ]
    );
}
#[test]
fn picker_tokens_material_and_voiceover_follow_every_accessibility_mode() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        for contrast in [false, true] {
            for transparency in [false, true] {
                for motion in [false, true] {
                    let t = Tokens::new(
                        scheme,
                        A11y {
                            increase_contrast: contrast,
                            reduce_transparency: transparency,
                            reduce_motion: motion,
                        },
                    );
                    let p = picker().plan(&t, true);
                    assert_eq!(p.width, t.metrics.popover_width);
                    assert_eq!(p.inset, t.metrics.inset);
                    assert_eq!(p.content_colour, t.colors.floating);
                    assert_eq!(p.sb_height, t.metrics.popover_width / 2.);
                    if transparency {
                        assert_eq!(p.material, pm_design::Material::Solid);
                    }
                    assert_eq!(
                        p.ax_labels,
                        [
                            "Saturation and brightness",
                            "Hue",
                            "Opacity",
                            "Pick a Colour From the Screen"
                        ]
                    );
                    assert_eq!(p.settings_tooltip, "Picker Settings");
                    assert_eq!(p.previous_tooltip, "Restore Previous Colour");
                    assert!(p.focus_trapped && p.previous_top_left);
                }
            }
        }
    }
}
#[test]
fn gamut_warning_is_specific_to_the_project_space() {
    let mut s = picker();
    assert!(s.plan(&tokens(), true).gamut_warning.is_none());
    s.outside_project_gamut = true;
    let p = s.plan(&tokens(), true);
    assert_eq!(
        p.gamut_warning,
        Some((
            "exclamationmark.triangle.fill",
            "Outside sRGB; saved as the nearest sRGB colour",
            tokens().colors.warning,
            tokens().metrics.symbol_inline
        ))
    );
    s.project_space = colour::Space::DisplayP3;
    assert!(s.plan(&tokens(), true).gamut_warning.is_none());
}
#[test]
fn previous_swatch_and_recents_only_change_on_explicit_actions() {
    let mut s = picker();
    s.recent = (0..10).map(|i| format!("#{i:06X}")).collect();
    assert_eq!(s.plan(&tokens(), true).recent.len(), 8);
    s.restore_previous();
    assert_eq!(s.current, "#112233");
    s.remember();
    s.remember();
    assert_eq!(s.recent.len(), 8);
    assert_eq!(s.recent[0], "#112233");
    assert_eq!(s.recent.iter().filter(|c| *c == "#112233").count(), 1);
}
#[test]
fn gradient_stop_and_midpoint_keyboard_routes_are_bounded() {
    use colour::{RampKey::*, midpoint_key, stop_key};
    assert_eq!(stop_key(50., Left, false), Some(49.));
    assert_eq!(stop_key(50., Right, true), Some(60.));
    assert_eq!(stop_key(50., Home, false), Some(0.));
    assert_eq!(stop_key(50., End, false), Some(100.));
    assert_eq!(stop_key(99., Right, true), Some(100.));
    assert_eq!(stop_key(1., Left, true), Some(0.));
    assert_eq!(midpoint_key(50., Left, false), Some(49.));
    assert_eq!(midpoint_key(1., Left, false), Some(1.));
    assert_eq!(midpoint_key(99., Right, false), Some(99.));
    assert_eq!(midpoint_key(50., Home, false), Some(1.));
    assert_eq!(midpoint_key(50., End, false), Some(99.));
    assert_eq!(stop_key(f64::NAN, Left, false), None);
}
#[test]
fn gradient_edits_address_ids_and_preserve_authored_order() {
    let mut ramp = colour::Ramp {
        stops: vec![
            colour::Stop {
                id: "a".into(),
                percent: 20.,
                colour: "#112233".into(),
            },
            colour::Stop {
                id: "b".into(),
                percent: 80.,
                colour: "#445566".into(),
            },
        ],
        selected: Some("a".into()),
    };
    assert!(ramp.move_stop("a", 90.));
    assert_eq!(ramp.stops[0].id, "a");
    assert_eq!(ramp.selected.as_deref(), Some("a"));
    assert!(!ramp.move_stop("missing", 20.));
    assert!(!ramp.move_stop("a", f64::INFINITY));
    assert!(ramp.remove("a"));
    assert_eq!(ramp.selected.as_deref(), Some("b"));
    assert!(ramp.remove("b"));
    assert!(ramp.selected.is_none());
}
#[test]
fn inspect_host_records_platform_requests_without_running_js() {
    let mut host = InspectHost::default();
    host.configure_text("field", text::FieldKind::Number.policy())
        .unwrap();
    host.colour_popover("swatch", picker().plan(&tokens(), true))
        .unwrap();
    host.sample_colour(42).unwrap();
    host.copy_colour("oklab(50% 0.1 0.2)").unwrap();
    assert_eq!(host.samples, [42]);
    assert_eq!(host.popovers[0].0, "swatch");
    assert_eq!(host.clipboard.as_deref(), Some("oklab(50% 0.1 0.2)"));
}
#[test]
fn real_kernel_text_commit_is_exact_and_undoable() {
    let (_a, k) = actor();
    let t = target(&k, "T");
    submit(&k, &t, t.set_text("日本語\nAuthored 🎬".into(), 0.));
    assert_eq!(
        layer(&k, "T")
            .get("d")
            .unwrap()
            .get("text")
            .unwrap()
            .as_str(),
        Some("日本語\nAuthored 🎬")
    );
    k.handle
        .call("P", Request::new(Caller::native(), HistoryCall::Undo))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(
        layer(&k, "T")
            .get("d")
            .unwrap()
            .get("text")
            .unwrap()
            .as_str(),
        Some("Original 🎬")
    );
}
#[test]
fn real_kernel_paragraph_resize_keeps_size_and_other_fields() {
    let (_a, k) = actor();
    let before = layer(&k, "T");
    let t = target(&k, "T");
    submit(&k, &t, t.paragraph(83.5, Some(42.), 0.));
    let after = layer(&k, "T");
    assert_eq!(
        after.get("d").unwrap().get("size"),
        before.get("d").unwrap().get("size")
    );
    assert_eq!(
        after
            .get("d")
            .unwrap()
            .get("boxWidth")
            .unwrap()
            .get("v")
            .unwrap()
            .as_f64(),
        Some(83.5)
    );
}
#[test]
fn real_kernel_locked_layer_refuses_and_stale_target_never_overwrites() {
    let (_a, k) = actor();
    let locked = target(&k, "L");
    let before = layer(&k, "L");
    assert!(matches!(
        k.submit(&locked, locked.set_text("Lost".into(), 0.))
            .unwrap()
            .wait()
            .unwrap()
            .unwrap()
            .reply,
        Reply::Throw(_)
    ));
    assert_eq!(layer(&k, "L"), before);
    let t = target(&k, "T");
    submit(&k, &t, t.set_text("First".into(), 0.));
    assert!(matches!(
        k.submit(&t, t.set_text("Stale".into(), 0.)),
        Err(kernel::AdmissionError::Stale)
    ));
    assert_eq!(
        layer(&k, "T")
            .get("d")
            .unwrap()
            .get("text")
            .unwrap()
            .as_str(),
        Some("First")
    );
}
#[test]
fn real_kernel_paths_are_atomic_and_undoable_by_id() {
    let (_a, k) = actor();
    let before = layer(&k, "S");
    let t = target(&k, "S");
    submit(
        &k,
        &t,
        t.add_path_vertex(kernel::NewVertex {
            path: "path",
            vertex: "v",
            mask: None,
            new_path: true,
            x: 1.25,
            y: -2.5,
        }),
    );
    let after = layer(&k, "S");
    let paths = after
        .get("d")
        .unwrap()
        .get("paths")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(
        paths[0].get("vertices").unwrap().as_array().unwrap().len(),
        1
    );
    k.handle
        .call("P", Request::new(Caller::native(), HistoryCall::Undo))
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(layer(&k, "S"), before);
}
#[test]
fn real_kernel_animator_projection_keeps_opaque_data_and_disabled_state() {
    let (_a, k) = actor();
    let t = target(&k, "T");
    let state = k.animators(&t).unwrap();
    assert_eq!(state.items.len(), 1);
    assert!(!state.items[0].enabled);
    assert!(state.items[0].animated);
    assert_eq!(
        state.items[0]
            .source
            .get("opaque")
            .unwrap()
            .get("extensionValue")
            .unwrap()
            .as_f64(),
        Some(42.)
    );
}
#[test]
fn unimplemented_animator_commands_do_not_claim_success_or_change_data() {
    let (_a, k) = actor();
    let t = target(&k, "T");
    let before = layer(&k, "T");
    for request in [
        t.add_animator("a", parse(r#"{"id":"a"}"#).unwrap()),
        t.remove_animator("ta1"),
        t.style_text("style", parse(r#"{"start":0,"end":2}"#).unwrap()),
    ] {
        assert!(matches!(
            k.submit(&t, request)
                .unwrap()
                .wait()
                .unwrap()
                .unwrap()
                .reply,
            Reply::Throw(_)
        ));
        assert_eq!(layer(&k, "T"), before);
    }
}
#[test]
fn real_kernel_preview_cancel_restores_original_and_commit_is_one_undo() {
    use pm_kernel::commands::{PropertyCommand, properties::SetProperty};
    for dismiss in [
        colour::Dismiss::Escape,
        colour::Dismiss::OutsideClick,
        colour::Dismiss::NotationReturn,
    ] {
        let (_a, k) = actor();
        let before = layer(&k, "T");
        let t = target(&k, "T");
        let ack = submit(&k, &t, t.begin_preview("Size"));
        let Reply::Gesture(gesture) = ack.reply else {
            panic!("expected gesture")
        };
        let preview = kernel::Preview { gesture };
        for v in [80., 96.] {
            k.handle
                .call(
                    "P",
                    preview.step(vec![
                        PropertyCommand::Set(SetProperty::new("T", "c.size", v, 0.)).into(),
                    ]),
                )
                .wait()
                .unwrap()
                .unwrap();
        }
        k.handle
            .call("P", preview.finish(dismiss))
            .wait()
            .unwrap()
            .unwrap();
        if dismiss.cancel() {
            assert_eq!(layer(&k, "T"), before);
        } else {
            k.handle
                .call("P", Request::new(Caller::native(), HistoryCall::Undo))
                .wait()
                .unwrap()
                .unwrap();
            assert_eq!(layer(&k, "T"), before);
        }
    }
}
#[test]
fn token_patches_flow_to_picker_and_dark_path_without_literal_defaults() {
    let mut t = tokens();
    t.metrics.popover_width = 300.;
    t.apply_patch([("colors.accent", 0xabcdef as f64)]).unwrap();
    assert_eq!(picker().plan(&t, true).width, 300.);
    assert_eq!(path_state().plan(&t).stroke, 0xabcdef);
}
#[test]
fn no_duplicate_accessibility_ids_in_the_path_plan() {
    let p = path_state().plan(&tokens());
    let ids = p.vertices.iter().map(|v| &v.ax.id).collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), p.vertices.len());
}
#[test]
fn engine_transform_moves_geometry_without_scaling_display_handles_or_ax_pixels() {
    let mut state = path_state();
    state.to_display = Some([0., 2., -2., 0., 100., 50.]);
    let p = state.plan(&tokens());
    assert_eq!(
        p.vertices[2].display_position,
        path::Point { x: 104., y: 54. }
    );
    assert_eq!(
        p.vertices[2].display_outgoing,
        path::Point { x: 108., y: 62. }
    );
    assert_eq!(p.vertices[2].side, 7.);
    assert_eq!(p.vertices[2].ax.label, "Point 3 of 5, 2, -2 pixels");
}
#[test]
fn gradient_add_selects_the_native_evaluators_colour_and_rejects_duplicate_ids() {
    let mut ramp = colour::Ramp::default();
    let stop = colour::Stop {
        id: "new".into(),
        percent: 33.3,
        colour: "color(display-p3 1 0 0)".into(),
    };
    assert!(ramp.add(stop.clone()));
    assert_eq!(ramp.selected.as_deref(), Some("new"));
    assert_eq!(ramp.stops[0].colour, stop.colour);
    assert!(!ramp.add(stop));
}
#[test]
fn every_request_is_native_and_revision_guarded() {
    let (_a, k) = actor();
    let t = target(&k, "T");
    let request = t.property("c.size", 72.into(), 0.);
    assert_eq!(request.caller, Caller::native());
    let Call::Transact {
        expected_revision, ..
    } = request.call
    else {
        panic!("expected transaction")
    };
    assert_eq!(expected_revision, Some(t.revision));
}

#[test]
fn review_path_handles_follow_gizmo_tokens_instead_of_keyframe_arithmetic() {
    let mut t = tokens();
    t.metrics.gizmo_handle = 13.;
    t.metrics.tangent_end = 11.;
    t.metrics.gizmo_keyline = 3.;
    let mut s = path_state();
    s.hovered = Some("v0".into());
    let p = s.plan(&t);
    assert_eq!(p.vertices[0].side, 15.);
    assert_eq!(p.vertices[1].side, 13.);
    assert_eq!(p.tangent_diameter, 11.);
    assert_eq!(p.tangent_width, 3.);
    assert_eq!(p.vertices[0].stroke_width, 3.);
}
