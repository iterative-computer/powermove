//! Full key -> model-command -> effect tests. Host effects never execute IO here.
use pm_agents::persistence::Store;
use pm_panel_agentpanel::{
    Error,
    input::{Catalog, Intent, Key, slash_choices},
    interaction::{Submission, outside_composer_blurs},
    model::{EffectKind, Panel},
};
use serde_json::{Value, json};
use std::io;
struct Empty;
impl Store for Empty {
    fn get(&mut self, _: &str) -> io::Result<Option<Value>> {
        Ok(None)
    }
    fn set(&mut self, _: &str, _: &Value) -> io::Result<()> {
        Ok(())
    }
}
fn panel() -> Panel {
    Panel::load(&mut Empty, "p", "t", 0).unwrap()
}
fn key(s: &str) -> Key {
    Key {
        name: s.into(),
        ..Default::default()
    }
}
fn ids() -> Submission<'static> {
    Submission {
        run: "r",
        request: "q",
    }
}
#[test]
fn enter_sends_once_repeat_and_ime_keep_draft() {
    let mut p = panel();
    p.draft_mut("t").unwrap().insert_text("Hello", 1, false);
    let e = p
        .composer_key(
            "t",
            &Key {
                composing: true,
                ..key("Enter")
            },
            false,
            ids(),
            2,
        )
        .unwrap();
    assert!(e.effect.is_none());
    let e = p.composer_key("t", &key("Enter"), false, ids(), 3).unwrap();
    assert!(matches!(
        e.effect.unwrap().kind,
        EffectKind::Send { steer: false, .. }
    ));
    p.draft_mut("t").unwrap().insert_text("remain", 4, false);
    assert!(
        p.composer_key(
            "t",
            &Key {
                repeat: true,
                ..key("Enter")
            },
            false,
            ids(),
            5
        )
        .unwrap()
        .effect
        .is_none()
    );
    assert_eq!(p.session("t").unwrap().draft.request_text(), "remain");
}
#[test]
fn newline_and_local_undo_do_not_touch_project_history() {
    let mut p = panel();
    p.draft_mut("t").unwrap().insert_text("one", 1, false);
    p.composer_key(
        "t",
        &Key {
            shift: true,
            ..key("Enter")
        },
        false,
        ids(),
        2,
    )
    .unwrap();
    assert_eq!(p.session("t").unwrap().draft.request_text(), "one\n");
    p.composer_key(
        "t",
        &Key {
            meta: true,
            ..key("z")
        },
        false,
        ids(),
        3,
    )
    .unwrap();
    assert_eq!(p.session("t").unwrap().draft.request_text(), "one");
}
#[test]
fn slash_tab_and_pointer_share_choice_handler_and_escape_does_not_send() {
    let mut p = panel();
    p.focus_composer("t", true).unwrap();
    p.draft_mut("t").unwrap().insert_text("/attach", 1, false);
    let e = p.composer_key("t", &key("Tab"), false, ids(), 2).unwrap();
    assert!(matches!(e.effect.unwrap().kind, EffectKind::ChooseFiles));
    assert_eq!(p.session("t").unwrap().draft.request_text(), "");
    p.draft_mut("t").unwrap().insert_text("/settings", 3, false);
    assert!(matches!(
        p.choose_slash("t", "settings", 4).unwrap().unwrap().kind,
        EffectKind::OpenSettings
    ));
    p.draft_mut("t").unwrap().insert_text("/", 5, false);
    let e = p.composer_key("t", &key("Escape"), true, ids(), 6).unwrap();
    assert_eq!(e.route.intent, Intent::DismissMenu);
    assert!(e.effect.is_none());
    let e = p.composer_key("t", &key("Escape"), true, ids(), 7).unwrap();
    assert!(matches!(e.effect.unwrap().kind, EffectKind::Shell(_)));
}
#[test]
fn slash_model_prefix_and_value_choose_a_catalog_model() {
    let mut p = panel();
    p.set_catalog(
        "t",
        Catalog {
            models: vec![("m".into(), "Model".into())],
            efforts: vec!["high".into()],
            ..Default::default()
        },
    )
    .unwrap();
    p.set_model("t", "m".into(), "high".into()).unwrap();
    p.draft_mut("t").unwrap().insert_text("/model", 1, false);
    p.choose_slash("t", "model", 2).unwrap();
    assert_eq!(p.session("t").unwrap().draft.request_text(), "/model ");
    p.choose_slash("t", "model-m", 3).unwrap();
    assert_eq!(p.session("t").unwrap().settings.model, "m");
    assert_eq!(p.session("t").unwrap().draft.request_text(), "");
    assert_eq!(
        p.choose_slash("t", "unknown", 4).unwrap_err(),
        Error::UnknownId("unknown".into())
    );
}
#[test]
fn delayed_file_picker_targets_captured_thread_and_background_draft_is_untouched() {
    let mut p = panel();
    p.draft_mut("t").unwrap().insert_text("/attach", 1, false);
    let e = p.choose_slash("t", "attach", 2).unwrap().unwrap();
    p.draft_mut("t").unwrap().insert_text("old", 3, false);
    p.new_thread("other", 4).unwrap();
    p.draft_mut("other").unwrap().insert_text("new", 5, false);
    let file = pm_panel_agentpanel::composer::Attachment::from_record(
        json!({"id":"a","name":"A","dataBase64":""}),
    )
    .unwrap();
    p.add_attachments(&e.target.thread, vec![file], 6).unwrap();
    assert_eq!(p.session("other").unwrap().draft.request_text(), "new");
    assert_eq!(p.session("t").unwrap().draft.attachments().count(), 1);
}
#[test]
fn outside_primary_and_auxiliary_clicks_release_composer_focus() {
    assert!(outside_composer_blurs(0, true, false));
    assert!(outside_composer_blurs(1, true, false));
    assert!(!outside_composer_blurs(2, true, false));
    assert!(!outside_composer_blurs(0, true, true));
    assert!(!outside_composer_blurs(0, false, false));
}
#[test]
fn frozen_slash_oracle_matches_78_source_executions() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/slash-v1.2.0.json")).unwrap();
    assert_eq!(fixture["baseline"], "v1.2.0");
    let catalog = Catalog {
        models: vec![
            ("sol".into(), "Sol 6.1".into()),
            ("claude-sonnet".into(), "Sonnet".into()),
        ],
        providers: vec![
            ("chatgpt".into(), "ChatGPT".into()),
            ("claude".into(), "Claude".into()),
        ],
        efforts: vec!["low".into(), "high".into(), "xhigh".into()],
        claude: true,
    };
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 78);
    for case in cases {
        let phase = case["legacyPhase"].as_str().unwrap();
        let actual = slash_choices(
            case["draft"].as_str().unwrap(),
            &catalog,
            phase == "working",
            case["blocked"] == true || phase == "applying",
        )
        .into_iter()
        .map(|c| {
            let mut v = json!({"id":c.id,"action":format!("{:?}",c.action).to_lowercase()});
            if let Some(value) = c.value {
                v["value"] = json!(value);
            }
            v
        })
        .collect::<Vec<_>>();
        assert_eq!(Value::Array(actual), case["choices"], "{case}");
    }
}

#[test]
fn agent_shortcut_keeps_loose_modifiers_but_declines_fields_and_repeat() {
    let chord = Key {
        meta: true,
        shift: true,
        ..key("K")
    };
    assert!(pm_panel_agentpanel::input::agent_shortcut(&chord, false));
    assert!(pm_panel_agentpanel::input::agent_shortcut(
        &Key {
            alt: true,
            control: true,
            ..chord.clone()
        },
        false
    ));
    assert!(!pm_panel_agentpanel::input::agent_shortcut(&chord, true));
    assert!(!pm_panel_agentpanel::input::agent_shortcut(
        &Key {
            repeat: true,
            ..chord
        },
        false
    ));
}

#[test]
fn modified_delete_still_arms_tokens_while_plain_text_deletion_remains_native() {
    let mut p = panel();
    p.draft_mut("t").unwrap().insert_text("👩‍👩‍👧‍👦", 1, false);
    let route = p
        .composer_key("t", &key("Backspace"), false, ids(), 2)
        .unwrap()
        .route;
    assert_eq!(route.intent, Intent::NativeText);
    assert!(!route.prevent_default);
    assert_eq!(p.session("t").unwrap().draft.request_text(), "👩‍👩‍👧‍👦");
    p.add_attachments(
        "t",
        vec![
            pm_panel_agentpanel::composer::Attachment::from_record(
                json!({"id":"a","name":"A","dataBase64":""}),
            )
            .unwrap(),
        ],
        3,
    )
    .unwrap();
    let route = p
        .composer_key(
            "t",
            &Key {
                alt: true,
                ..key("Backspace")
            },
            false,
            ids(),
            4,
        )
        .unwrap()
        .route;
    assert!(route.prevent_default);
    assert_eq!(p.session("t").unwrap().draft.armed(), Some("a"));
    p.composer_key(
        "t",
        &Key {
            control: true,
            ..key("Backspace")
        },
        false,
        ids(),
        5,
    )
    .unwrap();
    assert_eq!(p.session("t").unwrap().draft.attachments().count(), 0);
}
