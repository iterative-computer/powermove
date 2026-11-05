//! Compilation/translation coverage, not a GPU pixel parity gate.
#[test]
fn every_main_effect_revision_translates_through_the_native_pipeline() {
    let data: serde_json::Value =
        serde_json::from_str(include_str!("../../pm-fx-defs/src/main-effects.json")).unwrap();
    let shaders = data["shaders"].as_array().unwrap();
    assert_eq!(shaders.len(), 53);
    let mut failures = Vec::new();
    for shader in shaders {
        let id = shader["id"].as_str().unwrap();
        let version = shader["version"].as_u64().unwrap();
        let source = shader["source"].as_str().unwrap();
        match pm_glsl::prepare(source, 0).and_then(|prepared| prepared.finish()) {
            Ok(_) => {}
            Err(error) => failures.push(format!("{id}@{version}: {error:?}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
