//! Tagged focus routes and semantic view policies; no geometry assertions.
use pm_panel_agentpanel::{input::*, view::*};
fn key(name: &str) -> Key {
    Key {
        name: name.into(),
        ..Key::default()
    }
}
#[test]
fn composer_keys_never_leak_into_editor_commands() {
    let cases = [
        ("Enter", Intent::Submit),
        ("Escape", Intent::BlurComposer),
        ("ArrowDown", Intent::NativeText),
        ("x", Intent::NativeText),
    ];
    for (name, expected) in cases {
        let r = composer(&key(name), ComposerContext::default());
        assert_eq!(r.intent, expected);
        assert!(r.stop_editor);
    }
    let mut k = key("Enter");
    k.shift = true;
    assert_eq!(
        composer(&k, ComposerContext::default()).intent,
        Intent::Newline
    );
    k.repeat = true;
    assert_eq!(
        composer(&k, ComposerContext::default()).intent,
        Intent::Ignore
    );
}
#[test]
fn ime_and_229_and_disabled_never_submit() {
    for (ime, code, disabled) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let k = Key {
            composing: ime,
            code_229: code,
            ..key("Enter")
        };
        assert_ne!(
            composer(
                &k,
                ComposerContext {
                    disabled,
                    ..Default::default()
                }
            )
            .intent,
            Intent::Submit
        );
    }
}
#[test]
fn local_primary_chords_and_native_paste_keep_exact_modifier_rules() {
    for ctrl in [false, true] {
        let mut k = Key {
            control: ctrl,
            meta: !ctrl,
            ..key("a")
        };
        assert_eq!(
            composer(&k, ComposerContext::default()).intent,
            Intent::SelectAll
        );
        k.name = "v".into();
        let r = composer(&k, ComposerContext::default());
        assert_eq!(r.intent, Intent::NativePaste);
        assert!(!r.prevent_default);
        k.shift = true;
        assert_eq!(
            composer(&k, ComposerContext::default()).intent,
            Intent::NativeText
        );
        k.name = "z".into();
        assert_eq!(
            composer(&k, ComposerContext::default()).intent,
            Intent::Redo
        );
        k.shift = false;
        assert_eq!(
            composer(&k, ComposerContext::default()).intent,
            Intent::Undo
        );
    }
}
#[test]
fn slash_menu_takes_precedence_and_escape_closes_only_one_layer() {
    let context = ComposerContext {
        menu_open: true,
        popover_open: true,
        disabled: false,
    };
    for (name, intent) in [
        ("Enter", Intent::MenuChoose),
        ("Tab", Intent::MenuChoose),
        ("ArrowUp", Intent::MenuStep(-1)),
        ("ArrowDown", Intent::MenuStep(1)),
        ("Escape", Intent::DismissMenu),
    ] {
        assert_eq!(composer(&key(name), context).intent, intent);
    }
    assert_eq!(
        composer(
            &key("Escape"),
            ComposerContext {
                menu_open: false,
                ..context
            }
        )
        .intent,
        Intent::ClosePopover
    );
}
#[test]
fn slash_choices_follow_phase_and_provider_catalog_without_new_copy() {
    let catalog = Catalog {
        models: vec![("sol".into(), "Sol 6.1".into())],
        providers: vec![("claude".into(), "Claude".into())],
        efforts: vec!["high".into()],
        model_efforts: vec![],
        claude: true,
    };
    assert_eq!(slash_choices("/", &catalog, false, false).len(), 6);
    assert_eq!(slash_choices("/", &catalog, true, false).len(), 7);
    assert!(
        !slash_choices("/", &catalog, true, true)
            .iter()
            .any(|c| c.action == SlashAction::New)
    );
    assert_eq!(
        slash_choices("/model SOL", &catalog, false, false)[0]
            .value
            .as_deref(),
        Some("sol")
    );
    assert_eq!(
        slash_choices("/model claude-custom-v1", &catalog, false, false)[0]
            .value
            .as_deref(),
        Some("claude-custom-v1")
    );
    for draft in [" /model", "/model\nsol", "/model claude-a_b", "normal"] {
        assert!(slash_choices(draft, &catalog, false, false).is_empty());
    }
}
#[test]
fn slash_dismissal_resets_on_edit_and_navigation_wraps() {
    let choices = slash_choices("/", &Catalog::default(), false, false);
    let mut m = SlashMenu::default();
    m.focused = true;
    assert!(m.visible("/", false, &choices));
    m.step(-1, choices.len());
    assert_eq!(m.index, choices.len() - 1);
    m.step(1, choices.len());
    assert_eq!(m.index, 0);
    m.dismiss("/".into());
    assert!(!m.visible("/", false, &choices));
    m.changed();
    assert!(m.visible("/", false, &choices));
    assert!(!m.visible("/", true, &choices));
}
#[test]
fn question_enter_requires_unshifted_noncomposing_input() {
    assert!(question_submits(&key("Enter")));
    assert!(!question_submits(&Key {
        shift: true,
        ..key("Enter")
    }));
    assert!(!question_submits(&Key {
        composing: true,
        ..key("Enter")
    }));
}
fn threads(n: usize) -> Vec<ThreadSummary> {
    (0..n)
        .map(|i| ThreadSummary {
            id: format!("t{i}"),
            title: format!("Thread {i}"),
            busy: false,
            updated_at: 0,
        })
        .collect()
}
#[test]
fn thread_picker_search_navigation_pointer_choose_and_dismissal() {
    let mut p = ThreadPicker::default();
    p.open(threads(3), false).unwrap();
    p.step(-1);
    assert_eq!(p.focus, PickerFocus::Thread("t2".into()));
    p.step(1);
    assert_eq!(p.focus, PickerFocus::Thread("t2".into()));
    p.search("THREAD 1".into());
    assert_eq!(p.count(), 1);
    assert_eq!(p.choose(None).unwrap(), "t1");
    assert!(!p.open);
    p.open(threads(3), false).unwrap();
    assert_eq!(p.choose(Some("t2")).unwrap(), "t2");
    p.close(true);
    assert_eq!(p.focus, PickerFocus::Trigger);
    p.close(false);
    assert_eq!(p.focus, PickerFocus::Outside);
    assert!(p.open(threads(1), true).is_err());
}
#[test]
fn picker_navigation_above_first_returns_to_search() {
    let mut p = ThreadPicker::default();
    p.open(threads(2), false).unwrap();
    p.step(1);
    p.step(-1);
    assert_eq!(p.focus, PickerFocus::Search);
    p.search("absent".into());
    p.step(1);
    assert!(p.choose(None).is_err());
}
#[test]
fn sixty_thousand_threads_return_only_requested_rows() {
    let mut p = ThreadPicker::default();
    p.open(threads(60_001), false).unwrap();
    let page = p.page(59_990..60_010);
    assert_eq!(page.len(), 11);
    assert_eq!(page[0].id, "t59990");
    p.search("Thread 60000".into());
    assert_eq!(p.count(), 1);
    assert_eq!(p.page(0..10)[0].id, "t60000");
}
#[test]
fn transcript_selection_suppresses_follow_until_released() {
    let mut f = Follow::default();
    assert!(f.should_follow(false));
    f.scroll(true);
    assert!(!f.should_follow(false));
    assert!(f.should_follow(true));
    f.pointer_selecting = true;
    assert!(!f.should_follow(true));
    f.pointer_selecting = false;
    f.has_text_selection = true;
    assert!(!f.should_follow(true));
    f.has_text_selection = false;
    f.jump();
    assert!(!f.show_jump);
    assert!(f.should_follow(false));
    f.switch_thread();
    assert!(!f.user_scrolled);
}
#[test]
fn provider_status_ignores_old_generation_and_keeps_gate_during_retry() {
    let mut c = Connection::new("chatgpt".into());
    c.receive(1, ConnectionStatus::Disconnected);
    assert!(c.gate());
    c.receive(1, ConnectionStatus::Checking);
    assert!(c.gate());
    let generation = c.switch("claude".into());
    assert!(!c.receive(1, ConnectionStatus::Connected));
    assert!(c.gate());
    c.receive(generation, ConnectionStatus::Connected);
    assert!(!c.gate());
    c.receive(generation, ConnectionStatus::Disconnected);
    assert!(c.setup(true, true, true, true));
    assert!(!c.setup(true, false, true, true));
}
#[test]
fn visual_expand_zoom_replay_and_close_restore_semantic_state() {
    let mut v = VisualState::default();
    v.expand();
    assert_eq!(v.scale(), 1.0);
    v.zoom(-100);
    assert_eq!(v.scale(), 0.5);
    v.zoom(100);
    assert_eq!(v.scale(), 2.0);
    v.replay += 1;
    v.close();
    assert!(!v.expanded);
    assert_eq!(v.scale(), 1.0);
    assert_eq!(v.replay, 1);
}

#[test]
fn thread_picker_keyboard_uses_local_search_and_row_activation() {
    let mut p = ThreadPicker::default();
    p.open(threads(3), false).unwrap();
    p.key(&key("ArrowDown")).unwrap();
    assert_eq!(p.key(&key("Space")).unwrap(), Some("t0".into()));
    p.open(threads(3), false).unwrap();
    p.search("Thread 2".into());
    assert_eq!(p.key(&key("Enter")).unwrap(), Some("t2".into()));
    p.open(threads(3), false).unwrap();
    p.key(&key("Tab")).unwrap();
    assert!(!p.open);
    assert_eq!(p.focus, PickerFocus::Trigger);
}
#[test]
fn provider_connection_actions_are_generation_bound_and_duplicate_safe() {
    let mut c = Connection::new("claude".into());
    let request = c.begin(false).unwrap();
    assert_eq!(request.action, ConnectionAction::Connect);
    assert!(c.begin(false).is_err());
    c.receive(request.generation, ConnectionStatus::Unavailable);
    let retry = c.begin(true).unwrap();
    assert_eq!(retry.action, ConnectionAction::Retry);
    assert!(c.gate());
    c.switch("compatible".into());
    assert_eq!(
        c.begin(false).unwrap().action,
        ConnectionAction::OpenSettings
    );
}
#[test]
fn each_model_offers_its_own_efforts() {
    let catalog = Catalog {
        models: vec![
            ("fast".into(), "Fast".into()),
            ("deep".into(), "Deep".into()),
        ],
        providers: vec![],
        efforts: vec!["".into(), "low".into(), "xhigh".into()],
        model_efforts: vec![
            ("fast".into(), vec!["low".into()]),
            ("deep".into(), vec!["low".into(), "xhigh".into()]),
        ],
        claude: false,
    };
    assert_eq!(catalog.efforts_for("fast"), vec!["", "low"]);
    assert_eq!(catalog.efforts_for("deep"), vec!["", "low", "xhigh"]);
    // The provider's default model takes any listed effort.
    assert_eq!(catalog.efforts_for(""), vec!["", "low", "xhigh"]);
}
