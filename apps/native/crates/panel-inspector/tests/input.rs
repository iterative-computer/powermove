use pm_panel_inspector::{input::*, *};
#[test]
fn arithmetic_absolute_negative_and_precedence() {
    assert_eq!(parse_draft("-2", -2.0).unwrap(), -2.0);
    assert_eq!(parse_draft("2*(3+4)", 10.0).unwrap(), 14.0);
    assert_eq!(parse_draft(".5e2 + 1", 0.0).unwrap(), 51.0);
}
#[test]
fn arithmetic_relative_forms() {
    for (s, want) in [("+5", 15.0), ("*2", 20.0), ("/4", 2.5), ("-=3", 7.0)] {
        assert_eq!(parse_draft(s, 10.0).unwrap(), want);
    }
}
#[test]
fn arithmetic_rejects_nonfinite_javascript_and_partial_input() {
    for s in [
        "",
        "NaN",
        "1/0",
        "1+",
        "1;alert(1)",
        "Math.PI",
        "1e9999",
        "2x",
        "(2",
    ] {
        assert!(parse_draft(s, 0.0).is_err(), "{s}");
    }
}
#[test]
fn arithmetic_has_bounded_recursion_and_length() {
    assert!(parse_draft(&"(".repeat(1000), 0.0).is_err());
    assert!(parse_draft(&"1".repeat(5000), 0.0).is_err());
}
#[test]
fn click_opens_editor_and_drag_waits_for_threshold() {
    let mut i = NumericInput::new(NumericSpec::default(), 10.0).unwrap();
    i.pointer_begin(true);
    assert_eq!(
        i.pointer_move(2.0, 0.0, Modifiers::default()),
        InputEffect::Decline
    );
    assert_eq!(i.pointer_end(), InputEffect::OpenEditor);
    assert!(i.draft.is_some());
}
#[test]
fn drag_right_and_up_increases_value() {
    let mut i = NumericInput::new(NumericSpec::default(), 10.0).unwrap();
    i.pointer_begin(true);
    assert_eq!(
        i.pointer_move(10.0, -10.0, Modifiers::default()),
        InputEffect::Write(20.0)
    );
    assert_eq!(i.pointer_end(), InputEffect::Commit);
}
#[test]
fn drag_modifiers_shift_wins_over_option() {
    for (mods, want) in [
        (
            Modifiers {
                shift: true,
                ..Default::default()
            },
            60.0,
        ),
        (
            Modifiers {
                alt: true,
                ..Default::default()
            },
            10.5,
        ),
        (
            Modifiers {
                shift: true,
                alt: true,
                ..Default::default()
            },
            60.0,
        ),
    ] {
        let mut i = NumericInput::new(NumericSpec::default(), 10.0).unwrap();
        i.pointer_begin(true);
        assert_eq!(i.pointer_move(10.0, 0.0, mods), InputEffect::Write(want));
    }
}
#[test]
fn drag_is_from_start_and_escape_cancels() {
    let mut i = NumericInput::new(NumericSpec::default(), 10.0).unwrap();
    i.pointer_begin(true);
    assert_eq!(
        i.pointer_move(10.0, 0.0, Modifiers::default()),
        InputEffect::Write(15.0)
    );
    assert_eq!(
        i.pointer_move(20.0, 0.0, Modifiers::default()),
        InputEffect::Write(20.0)
    );
    assert_eq!(i.key("Escape", Modifiers::default()), InputEffect::Cancel);
    assert_eq!(i.pointer_end(), InputEffect::Decline);
}
#[test]
fn drag_clamps_and_nonprimary_button_declines() {
    let mut i = NumericInput::new(
        NumericSpec {
            min: Some(0.0),
            max: Some(20.0),
            ..Default::default()
        },
        10.0,
    )
    .unwrap();
    i.pointer_begin(false);
    assert_eq!(
        i.pointer_move(100.0, 0.0, Modifiers::default()),
        InputEffect::Decline
    );
    i.pointer_begin(true);
    assert_eq!(
        i.pointer_move(100.0, 0.0, Modifiers::default()),
        InputEffect::Write(20.0)
    );
    assert_eq!(
        i.pointer_move(-100.0, 0.0, Modifiers::default()),
        InputEffect::Write(0.0)
    );
}
#[test]
fn horizontal_wheel_accumulates_fractional_travel() {
    let mut i = NumericInput::new(NumericSpec::default(), 10.0).unwrap();
    assert!(i.wheel_begin());
    assert_eq!(i.wheel(1.0, Modifiers::default()), InputEffect::Write(10.5));
    assert_eq!(i.wheel(1.0, Modifiers::default()), InputEffect::Write(11.0));
    assert_eq!(i.wheel_end(), InputEffect::Commit);
}
#[test]
fn wheel_at_limit_declines_without_losing_cancel() {
    let mut i = NumericInput::new(
        NumericSpec {
            max: Some(10.0),
            ..Default::default()
        },
        10.0,
    )
    .unwrap();
    i.wheel_begin();
    assert_eq!(i.wheel(1.0, Modifiers::default()), InputEffect::Decline);
    assert_eq!(i.cancel(), InputEffect::Cancel);
}
#[test]
fn enter_f2_and_typed_commit() {
    for key in ["Enter", "F2"] {
        let mut i = NumericInput::new(NumericSpec::default(), 10.0).unwrap();
        assert_eq!(i.key(key, Modifiers::default()), InputEffect::OpenEditor);
        i.set_draft("*2".into());
        assert_eq!(
            i.key("Enter", Modifiers::default()),
            InputEffect::Write(20.0)
        );
    }
}
#[test]
fn unchanged_or_invalid_draft_creates_no_write() {
    for draft in ["10", "bad", "1/0"] {
        let mut i = NumericInput::new(NumericSpec::default(), 10.0).unwrap();
        i.open();
        i.set_draft(draft.into());
        assert_eq!(i.finish_draft(true), InputEffect::Commit);
    }
}
#[test]
fn typed_value_is_not_silently_clamped() {
    let mut i = NumericInput::new(
        NumericSpec {
            max: Some(20.0),
            ..Default::default()
        },
        10.0,
    )
    .unwrap();
    i.open();
    i.set_draft("30".into());
    assert_eq!(i.finish_draft(true), InputEffect::Write(30.0));
}
#[test]
fn draft_arrow_steps_without_document_write_or_clamp() {
    let mut i = NumericInput::new(
        NumericSpec {
            max: Some(20.0),
            ..Default::default()
        },
        10.0,
    )
    .unwrap();
    i.open();
    i.set_draft("30px".into());
    assert_eq!(
        i.key(
            "ArrowUp",
            Modifiers {
                shift: true,
                ..Default::default()
            }
        ),
        InputEffect::Decline
    );
    assert_eq!(i.draft, Some("40".into()));
    assert_eq!(i.key("Escape", Modifiers::default()), InputEffect::Cancel);
}
#[test]
fn readout_arrows_step_shift_and_clamp() {
    let mut i = NumericInput::new(
        NumericSpec {
            max: Some(15.0),
            ..Default::default()
        },
        10.0,
    )
    .unwrap();
    assert_eq!(
        i.key(
            "ArrowUp",
            Modifiers {
                shift: true,
                ..Default::default()
            }
        ),
        InputEffect::Write(15.0)
    );
    assert_eq!(
        i.key(
            "ArrowDown",
            Modifiers {
                alt: true,
                ..Default::default()
            }
        ),
        InputEffect::Write(14.0)
    );
}
#[test]
fn slider_arrows_home_end() {
    let s = NumericSpec {
        step: 2.0,
        min: Some(0.0),
        max: Some(100.0),
        ..Default::default()
    };
    assert_eq!(s.slider_key(50.0, "Home", Modifiers::default()), Some(0.0));
    assert_eq!(s.slider_key(50.0, "End", Modifiers::default()), Some(100.0));
    assert_eq!(
        s.slider_key(
            50.0,
            "ArrowLeft",
            Modifiers {
                shift: true,
                ..Default::default()
            }
        ),
        Some(30.0)
    );
}
#[test]
fn toggle_direction_space_enter_and_disabled() {
    assert_eq!(toggle_key(true, "ArrowLeft", false), Some(false));
    assert_eq!(toggle_key(false, "ArrowRight", false), Some(true));
    assert_eq!(toggle_key(true, " ", false), Some(false));
    assert_eq!(toggle_key(false, "Enter", false), Some(true));
    assert_eq!(toggle_key(false, "Enter", true), None);
}
#[test]
fn invalid_numeric_specs_are_refused() {
    for s in [
        NumericSpec {
            step: 0.0,
            ..Default::default()
        },
        NumericSpec {
            speed: f64::NAN,
            ..Default::default()
        },
        NumericSpec {
            min: Some(10.0),
            max: Some(0.0),
            ..Default::default()
        },
    ] {
        assert!(NumericInput::new(s, 0.0).is_err());
    }
}
#[test]
fn playback_clock_limits_to_15hz_and_pause_is_exact() {
    let mut c = ReadoutClock::default();
    assert!(c.update(0.0, 1.0, true, false));
    assert!(!c.update(10.0, 1.01, true, false));
    assert_eq!(c.time, 1.0);
    assert!(c.update(67.0, 1.067, true, false));
    assert!(c.update(68.0, 1.068, false, false));
    assert_eq!(c.time, 1.068);
    assert!(c.update(69.0, 1.069, true, true));
    assert_eq!(c.time, 1.069);
}
#[test]
fn scroll_memory_is_selection_scoped_and_has_no_geometry() {
    let mut s = ScrollMemory::default();
    s.remember(vec!["a".into()], "opacity".into(), 0.25)
        .unwrap();
    assert_eq!(s.restore(&["a".into()]), Some(&("opacity".into(), 0.25)));
    assert_eq!(s.restore(&["b".into()]), None);
    assert!(s.remember(vec![], "x".into(), f64::NAN).is_err());
}
#[test]
fn effect_keyboard_routes_before_layer_shortcuts() {
    let mods = Modifiers {
        meta: true,
        ..Default::default()
    };
    assert_eq!(
        EffectUiAction::key("c", mods, false),
        KeyDisposition::Consume(EffectUiAction::Copy)
    );
    assert_eq!(
        EffectUiAction::key("v", mods, false),
        KeyDisposition::Consume(EffectUiAction::Paste)
    );
    assert_eq!(
        EffectUiAction::key("Delete", Modifiers::default(), false),
        KeyDisposition::Consume(EffectUiAction::Delete)
    );
    assert_eq!(
        EffectUiAction::key("c", mods, true),
        KeyDisposition::Decline
    );
    assert_eq!(
        EffectUiAction::key("x", mods, false),
        KeyDisposition::Decline
    );
}
#[test]
fn effect_option_arrows_reorder_and_plain_arrows_navigate() {
    assert_eq!(
        EffectUiAction::key(
            "ArrowUp",
            Modifiers {
                alt: true,
                ..Default::default()
            },
            false
        ),
        KeyDisposition::Consume(EffectUiAction::Reorder(-1))
    );
    assert_eq!(
        EffectUiAction::key("ArrowDown", Modifiers::default(), false),
        KeyDisposition::Consume(EffectUiAction::Navigate(1))
    );
}
#[test]
fn effect_navigation_extends_range_and_clamps_at_edges() {
    let mut s = EffectSelection::default();
    let ids = vec!["a".into(), "b".into(), "c".into()];
    s.select("L", &ids, "a", Modifiers::default()).unwrap();
    s.navigate(&ids, "a", 1, true).unwrap();
    assert_eq!(s.ids, ["a", "b"]);
    s.navigate(&ids, "b", 1, true).unwrap();
    assert_eq!(s.ids, ["a", "b", "c"]);
    s.navigate(&ids, "c", 1, false).unwrap();
    assert_eq!(s.ids, ["a", "b", "c"]);
}
