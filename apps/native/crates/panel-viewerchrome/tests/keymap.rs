use pm_model::Value;
use pm_panel_viewerchrome::{input::*, *};
fn event(key: &str, modifiers: Modifiers) -> KeyEvent<'_> {
    KeyEvent {
        key,
        modifiers,
        repeat: false,
        in_field: false,
        cancelled: false,
    }
}
fn assert_command(event: KeyEvent<'_>, id: &str, args: &[Value]) {
    match resolve_key(event, KeyContext::default()) {
        KeyRoute::Command {
            id: actual,
            args: actual_args,
            ..
        } => {
            assert_eq!(actual, id);
            assert_eq!(actual_args, args);
        }
        other => panic!("{event:?} -> {other:?}"),
    }
}
#[test]
fn every_viewer_transport_and_zoom_chord_preserves_tag_command_id() {
    let plain = Modifiers::default();
    let shift = Modifiers {
        shift: true,
        ..plain
    };
    for (key, id) in [
        ("space", "play"),
        ("home", "gotoStart"),
        ("end", "gotoEnd"),
        ("pageup", "prevFrame"),
        ("pagedown", "nextFrame"),
        ("j", "prevVisibleEvent"),
        ("k", "nextVisibleEvent"),
        ("i", "gotoLayerIn"),
        ("o", "gotoLayerOut"),
        ("b", "workIn"),
        ("n", "workOut"),
        (".", "zoomIn"),
        (",", "zoomOut"),
        ("/", "actualSize"),
        ("enter", "text.editSelected"),
    ] {
        assert_command(event(key, plain), id, &[]);
    }
    for (key, id) in [
        ("home", "gotoWorkIn"),
        ("end", "gotoWorkOut"),
        ("j", "timeline.adjacentKeyframe:prev"),
        ("k", "timeline.adjacentKeyframe:next"),
        ("b", "workIn"),
        ("n", "workOut"),
        ("/", "fitComposition"),
        ("?", "fitComposition"),
    ] {
        assert_command(event(key, shift), id, &[]);
    }
    for (key, frames) in [("pageup", -10.0), ("pagedown", 10.0)] {
        assert_command(event(key, shift), "stepFrames", &[Value::Number(frames)]);
    }
    for primary in [
        Modifiers {
            meta: true,
            ..plain
        },
        Modifiers {
            control: true,
            ..plain
        },
    ] {
        for (key, id) in [
            ("=", "zoomIn"),
            ("-", "zoomOut"),
            ("0", "fitComposition"),
            ("1", "actualSize"),
        ] {
            assert_command(event(key, primary), id, &[]);
        }
        let shifted = Modifiers {
            shift: true,
            ..primary
        };
        assert_command(event("+", shifted), "zoomIn", &[]);
        assert_command(event("h", shifted), "toggleLayerControls", &[]);
        assert_command(
            event("left", shifted),
            "stepFrames",
            &[Value::Number(-10.0)],
        );
        assert_command(
            event("right", shifted),
            "stepFrames",
            &[Value::Number(10.0)],
        );
        assert_command(
            event("left", primary),
            if primary.meta {
                "prevFrame"
            } else {
                "timeline.adjacentKeyframe:prev"
            },
            &[],
        );
        assert_command(
            event("right", primary),
            if primary.meta {
                "nextFrame"
            } else {
                "timeline.adjacentKeyframe:next"
            },
            &[],
        );
    }
}
#[test]
fn strict_extra_modifiers_field_and_precancel_decline() {
    for key in [".", "home", "j", "pageup", "enter"] {
        for modifiers in [
            Modifiers {
                alt: true,
                ..Default::default()
            },
            Modifiers {
                meta: true,
                control: true,
                ..Default::default()
            },
        ] {
            assert_eq!(
                resolve_key(event(key, modifiers), KeyContext::default()),
                KeyRoute::Declined
            );
        }
        let mut ev = event(key, Modifiers::default());
        ev.in_field = true;
        assert_eq!(resolve_key(ev, KeyContext::default()), KeyRoute::Declined);
        ev.in_field = false;
        ev.cancelled = true;
        assert_eq!(resolve_key(ev, KeyContext::default()), KeyRoute::Declined);
    }
}
#[test]
fn repeat_allowed_only_for_frame_steps_not_play_zoom_or_key_navigation() {
    for (key, repeatable) in [
        ("pageup", true),
        ("pagedown", true),
        ("space", false),
        (".", false),
        ("j", false),
        ("home", false),
    ] {
        let mut ev = event(key, Modifiers::default());
        ev.repeat = true;
        assert_eq!(
            !matches!(resolve_key(ev, KeyContext::default()), KeyRoute::Declined),
            repeatable
        );
    }
}
#[test]
fn space_exposes_hand_and_still_routes_global_play_once() {
    let mut state = PointerState {
        over: true,
        ..Default::default()
    };
    let ev = event(
        "space",
        Modifiers {
            alt: true,
            shift: true,
            ..Default::default()
        },
    );
    state.key_down(ev);
    assert!(state.temporary_hand);
    assert_eq!(
        resolve_key(
            ev,
            KeyContext {
                pointer_over: true,
                ..Default::default()
            }
        ),
        KeyRoute::Command {
            id: "play",
            args: vec![],
            temporary_hand: true
        }
    );
    state.key_up("space");
    assert!(!state.temporary_hand);
    state.key_down(ev);
    state.blur();
    assert!(!state.temporary_hand);
    assert!(!state.over);
    let mut field = ev;
    field.in_field = true;
    state.over = true;
    state.key_down(field);
    assert!(!state.temporary_hand);
}
#[test]
fn source_slider_keys_and_escape_do_not_dispatch_editor_navigation() {
    let context = KeyContext {
        source_active: true,
        source_slider: true,
        ..Default::default()
    };
    for (key, command) in [
        ("left", SourceCommand::StepSeconds(-5.0)),
        ("right", SourceCommand::StepSeconds(5.0)),
        ("home", SourceCommand::Start),
        ("end", SourceCommand::End),
        ("escape", SourceCommand::Close),
    ] {
        assert_eq!(
            resolve_key(event(key, Modifiers::default()), context),
            KeyRoute::Action(Action::Host(HostAction::Source(command)))
        );
    }
    assert_eq!(
        resolve_key(
            event("escape", Modifiers::default()),
            KeyContext {
                active_drag: true,
                ..context
            }
        ),
        KeyRoute::CancelDrag
    );
}
#[test]
fn command_actions_use_native_plan_or_preserve_foreign_service_identity() {
    let viewport = Viewport {
        stage: Size {
            width: 800.0,
            height: 600.0,
        },
        composition: Size {
            width: 1920.0,
            height: 1080.0,
        },
        fit_inset: 8.0,
    };
    assert_eq!(
        command_action("prevFrame", &[], viewport).unwrap(),
        Action::Transport(TransportAction::Step(-1))
    );
    assert_eq!(
        command_action("workIn", &[], viewport).unwrap(),
        Action::WorkIn
    );
    assert_eq!(
        command_action("zoomIn", &[], viewport).unwrap(),
        Action::ZoomBy(1.25, viewport)
    );
    assert_eq!(
        command_action("timeline.adjacentKeyframe:prev", &[], viewport).unwrap(),
        Action::Host(HostAction::Command {
            id: "timeline.adjacentKeyframe:prev".into(),
            args: vec![]
        })
    );
    for args in [
        vec![],
        vec![Value::Number(0.0)],
        vec![Value::Number(1.5)],
        vec![Value::Number(f64::NAN)],
        vec![Value::Number(1.0), Value::Number(2.0)],
    ] {
        assert!(command_action("stepFrames", &args, viewport).is_err());
    }
}
