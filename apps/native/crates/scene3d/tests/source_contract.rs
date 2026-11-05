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
