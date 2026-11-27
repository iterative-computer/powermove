use pm_panel_agentpanel::{
    activity::Trace,
    placement::{self, Placement, Targets},
};
use serde_json::{Value, json};
#[test]
fn placement_split_matches_18_frozen_source_executions() {
    let fixture: Value =
        serde_json::from_str(include_str!("../fixtures/placement-v1.2.0.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 18);
    for case in cases {
        let got = placement::split(case["source"].as_str().unwrap(), case["streaming"] == true);
        assert_eq!(got.text, case["expected"]["text"], "{case}");
        assert_eq!(json!(got.messages), case["expected"]["messages"], "{case}");
    }
}
#[test]
fn placements_validate_ids_and_live_ownership_without_mutating_workspace() {
    let mut targets = Targets::default();
    targets.visible_panels.insert("p".into());
    targets.visible_docks.insert("d".into(), vec!["p".into()]);
    assert!(matches!(
        placement::parse(
            "POWERMOVE_UI_TARGET {\"kind\":\"panel\",\"id\":\"p\",\"label\":\"Work\",\"selector\":\".field\",\"insert\":\"after\"}",
            &targets
        ),
        Some(Placement::Panel { .. })
    ));
    for value in [
        json!({"kind":"panel","id":"missing","label":"Work"}),
        json!({"kind":"panel","id":"p","label":"Work","insert":"after"}),
        json!({"kind":"dock","id":"d","label":"Work","beforePanelId":"foreign"}),
        json!({"kind":"panel","id":"p","label":""}),
    ] {
        assert!(placement::parse(&format!("POWERMOVE_UI_TARGET {value}"), &targets).is_none());
    }
}
#[test]
fn metadata_stream_fragments_never_become_saved_reply_prose() {
    let mut trace = Trace::default();
    for (i, text) in [
        "Hello\nPOW",
        "ERMOVE_UI_TARGET {\"kind\":\"panel\",\"id\":\"p\",",
        "\"label\":\"Work\"}\nDone",
    ]
    .iter()
    .enumerate()
    {
        trace
            .receive(&json!({"kind":"answer","text":text}), i as u64)
            .unwrap();
    }
    assert_eq!(trace.rows.at(0).unwrap().value["text"], "Hello\n\nDone");
    assert_eq!(trace.placements.len(), 1);
    trace.seal(4).unwrap();
    assert!(
        !trace
            .rows
            .at(0)
            .unwrap()
            .value
            .to_string()
            .contains("POWERMOVE_UI_TARGET")
    );
    trace
        .receive(
            &json!({"kind":"thought","text":"POWERMOVE_UI_TARGET {\"id\":\"p\"}"}),
            5,
        )
        .unwrap();
    assert_eq!(trace.rows.len(), 1);
}
