use serde_json::Value;
fn canonical(v: &Value) -> Value {
    serde_json::from_str(&pm_model::json::to_canonical_string(&pm_scene3d::to_model(
        v,
    )))
    .unwrap()
}
#[test]
fn schema_matches_independent_owner_source_cases() {
    let fixture: Value = serde_json::from_str(include_str!("reference/cases.json")).unwrap();
    for (case, c) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let actual = pm_scene3d::schema::parse_scene(c["input"].clone());
        // Pinned Studio bd30e2037 deliberately replaces the old 16-light cap
        // with 4096 for environment rigs. Keep the original oracle fixture and
        // make this one intentional contract expansion explicit.
        if case == 29 {
            assert_eq!(actual.unwrap()["lights"].as_array().unwrap().len(), 17);
            continue;
        }
        if c["ok"] == true {
            assert_eq!(
                canonical(&actual.unwrap_or_else(|e| panic!("case {case}: {e}"))),
                canonical(&c["output"]),
                "source case {case}"
            );
        } else {
            assert!(actual.is_err(), "source refused case {case}");
        }
    }
}

#[test]
fn studio_light_tree_document_budget_is_4096() {
    let lights: Vec<_> = (0..4096)
        .map(|i| serde_json::json!({"id":format!("light{i}")}))
        .collect();
    assert!(pm_scene3d::schema::parse_scene(serde_json::json!({"lights":lights})).is_ok());
    let lights: Vec<_> = (0..4097)
        .map(|i| serde_json::json!({"id":format!("light{i}")}))
        .collect();
    assert!(pm_scene3d::schema::parse_scene(serde_json::json!({"lights":lights})).is_err());
}
