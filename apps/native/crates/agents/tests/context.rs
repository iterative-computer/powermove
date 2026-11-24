use pm_agents::context::*;
use serde_json::{Value, json};
#[test]
fn catalog_prioritizes_selection_then_mentioned_and_stays_bounded() {
    let layers:Vec<_>=(0..120).map(|i|json!({"id":format!("l{i}"),"name":format!("Layer {i} end"),"type":"text","p":{"text":"é".repeat(2000)}})).collect();
    let c = catalog(
        json!({"id":"comp"}),
        &layers,
        "Improve Layer 118 end",
        &["l119".into()],
    );
    assert_eq!(c["layers"][0]["id"], "l119");
    assert_eq!(c["layers"][1]["id"], "l118");
    assert!(c.to_string().chars().count() <= CATALOG_CHARS);
    assert!(c["layerIndex"].to_string().chars().count() <= INDEX_CHARS);
    assert_eq!(c["totalLayers"], 120);
    assert_eq!(c["truncated"], true);
    assert!(c["note"].as_str().unwrap().contains("nextLayerOffset"));
}
#[test]
fn oversized_layer_does_not_hide_later_editable_targets() {
    let c = catalog(
        json!({}),
        &[
            json!({"id":"huge","p":"x".repeat(90_000)}),
            json!({"id":"small"}),
        ],
        "",
        &[],
    );
    assert_eq!(c["layers"][0]["id"], "small");
}
#[test]
fn region_meaning_travels_with_pixels_and_fences_stale_documents() {
    let attachments = vec![
        json!({"name":"Frame.png","targetContext":{"projectId":"p","documentId":7,"windowId":2,"panelId":"viewer","time":1.5,"compositionTime":1.5,"coordinateSpace":"composition-pixels","area":[1,2,30,40],"layersUnderBox":["title"]}}),
    ];
    let text = targets(&attachments, "p", 7);
    for part in [
        "Frame.png",
        "viewer",
        "1.5",
        "compositionTime",
        "composition-pixels",
        "layersUnderBox",
        "title",
        "sameDocument\":true",
    ] {
        assert!(text.contains(part), "{part}");
    }
    assert!(targets(&attachments, "other", 8).contains("sameDocument\":false"));
    assert!(targets(&[], "p", 7).is_empty());
}
#[test]
fn ordinary_prompt_has_v1_task_creative_style_and_native_authoring_rules() {
    let p = assemble("Create a polished effect panel and theme");
    assert!(p.contains(
        "Add timeline markers only when requested or when a few sparse structural cues would help."
    ));
    for section in ["TASK FOCUS", "CREATIVE JUDGMENT", "RESPONSE STYLE"] {
        assert!(p.contains(section), "missing {section}");
    }
    assert!(!guidance("What is selected?").contains("CREATIVE JUDGMENT"));
    assert!(!p.contains("returned in extensions"));
}
#[test]
fn representative_frames_include_playhead_beginning_middle_end_without_duplicates() {
    assert_eq!(
        representative_times(2., 10., 30.),
        vec![2., 0., (10. - 1. / 30.) / 2., 10. - 1. / 30.]
    );
    assert_eq!(representative_times(0., 0., 30.), vec![0.]);
}
#[test]
fn layer_index_stays_bounded_for_many_long_names() {
    let layers: Vec<Value> = (0..900)
        .map(|i| json!({"id":i.to_string(),"name":"x".repeat(400),"type":"text"}))
        .collect();
    let c = catalog(json!({}), &layers, "", &["899".into()]);
    assert!(c.to_string().chars().count() <= CATALOG_CHARS);
    assert_eq!(c["layers"][0]["id"], "899");
    assert!(c["indexedLayers"].as_u64().unwrap() < 900);
}
#[test]
fn task_profile_uses_latest_request_instead_of_thread_history_or_observation() {
    let prompt = "THREAD CONTEXT\nCreate an effect panel\n\nUSER REQUEST\nWhat is selected?\n\nLIVE COMPOSITION\n{\"name\":\"create effect panel\"}";
    let p = guidance(prompt);
    assert!(!p.contains("CREATIVE JUDGMENT"));
    assert!(!p.contains("NATIVE PANEL DESIGN"));
}
#[test]
fn oversized_composition_metadata_cannot_break_catalog_budget() {
    let c = catalog(json!({"name":"x".repeat(100_000)}), &[], "", &[]);
    assert!(c.to_string().chars().count() <= CATALOG_CHARS);
}
#[test]
fn region_metadata_is_bounded_without_cutting_json() {
    let a = json!({"name":"Frame.png","targetContext":{"projectId":"p","panelId":"viewer","compositionTime":1.5,"coordinateSpace":"composition-pixels","area":[1,2,30,40],"layersUnderBox":["x".repeat(50_000)]}});
    let text = targets(&[a], "p", 1);
    assert!(text.len() < 2000);
    assert!(text.contains("metadataTruncated"));
    assert!(text.contains("viewer"));
    assert!(text.contains("compositionTime"));
    assert!(text.contains("layersUnderBoxCount"));
}

#[test]
fn ordinary_prompt_contains_testing_tool_effect_panel_and_theme_guidance() {
    let p = assemble("Create an effect panel and theme");
    for section in [
        "BACKGROUND TESTING",
        "TOOL RELIABILITY",
        "EFFECT REQUESTS",
        "NATIVE PANEL DESIGN",
        "REVERSIBLE THEMES",
        "publish_extensions",
        "panel_begin",
        "api.onDispose",
        "hot.dispose",
        "validate_effect",
    ] {
        assert!(p.contains(section), "missing {section}");
    }
    assert!(!guidance("What is selected?").contains("EXTENSION COMPLETION"));
    for request in ["Create a mod", "Build an integration", "Use the sandbox"] {
        assert!(
            guidance(request).contains("EXTENDING POWERMOVE"),
            "{request}"
        );
    }
}

#[test]
fn catalog_uses_v1_utf16_budgets_for_emoji() {
    let layers: Vec<_> = (0..80)
        .map(|i| json!({"id":i.to_string(),"name":"🎬".repeat(300),"p":{"text":"🎬".repeat(2000)}}))
        .collect();
    let c = catalog(json!({}), &layers, "", &["79".into()]);
    assert!(c.to_string().encode_utf16().count() <= CATALOG_CHARS);
    assert!(c["layerIndex"].to_string().encode_utf16().count() <= INDEX_CHARS);
    assert!(
        c["layerIndex"][0]["name"]
            .as_str()
            .unwrap()
            .encode_utf16()
            .count()
            <= 160
    );
    assert_eq!(c["layers"][0]["id"], "79");
}

#[test]
fn all_six_region_anchors_survive_large_layer_lists_within_budget() {
    let attached: Vec<_>=(0..6).map(|i|json!({"name":format!("Capture {i}.png"),"targetContext":{"projectId":"p","documentId":7,"panelId":format!("panel-{i}"),"compositionTime":i,"coordinateSpace":"window-points","area":[0,0,100,100],"layersUnderBox":(0..1000).map(|j|format!("layer-{j}")).collect::<Vec<_>>()}})).collect();
    let text = targets(&attached, "p", 7);
    assert!(text.encode_utf16().count() < 18_000);
    for i in 0..6 {
        assert!(text.contains(&format!("panel-{i}")));
    }
    assert!(text.contains("layersUnderBoxCount"));
}

#[test]
fn observation_preserves_history_and_keeps_the_latest_request_last() {
    let prompt = "THREAD CONTEXT\nEarlier design decision\n\nUSER REQUEST\nImprove this";
    let observed = with_observation(
        prompt,
        "\n\nLIVE COMPOSITION\n{\"name\":\"fixture\"}",
        "Improve this",
    );
    assert!(observed.starts_with("THREAD CONTEXT\nEarlier design decision"));
    assert!(observed.contains("LIVE COMPOSITION"));
    assert!(observed.ends_with("USER REQUEST\nImprove this"));
    assert_eq!(observed.matches("USER REQUEST").count(), 1);
    let question = with_observation(
        "What is selected?",
        "\n\nLIVE COMPOSITION\nCreate an effect panel",
        "What is selected?",
    );
    assert!(question.ends_with("USER REQUEST\nWhat is selected?"));
    assert!(!guidance(&question).contains("CREATIVE JUDGMENT"));
    assert_eq!(with_observation("plain", "", "plain"), "plain");
}

#[test]
fn routine_visual_edits_get_frames_without_changing_v1_creative_guidance() {
    assert!(visual_edit("Move selected layer left"));
    assert!(visual_edit("Delete this"));
    assert!(visual_edit("Add a circle"));
    assert!(!creative("Move selected layer left"));
    assert!(!visual_edit("What is selected?"));
}
