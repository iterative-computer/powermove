use pm_corpus_checks::shaders;
use pm_glsl::FailureStage;
use std::{fs, path::PathBuf};

fn helper() -> PathBuf {
    std::env::var_os("POWERMOVE_GLSL_COMPILER").expect("build pm-glsl-compiler and set POWERMOVE_GLSL_COMPILER; this test must use the real helper").into()
}

#[test]
fn seven_generated_shader_families_translate_with_relocated_real_helper() {
    let root = tempfile::tempdir().unwrap();
    let helper_copy = root.path().join("frozen-compiler");
    fs::copy(helper(), &helper_copy).unwrap();
    let builtins: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/builtin-shaders.json")).unwrap();
    // Synthetic coverage of the open shader class, not a replay of seven private entries.
    for id in [
        "glow",
        "shadow",
        "gradient",
        "blur",
        "displace",
        "color",
        "crossfade",
    ] {
        let d = builtins["definitions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .unwrap();
        shaders::compile(
            &helper_copy,
            d["glsl"]["source"].as_str().unwrap(),
            d["glsl"]["bodyLineOffset"].as_u64().unwrap() as u32,
        )
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    }
}

#[test]
fn real_diagnostics_distinguish_missing_helper_from_authored_compile_failure() {
    let source =
        "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main(){o=missing_symbol;}";
    let failure = shaders::compile(&helper(), source, 2).unwrap_err();
    assert_eq!(failure.stage, FailureStage::EsValidate);
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|d| d.line == Some(2) && d.message.contains("missing_symbol")),
        "{failure:?}"
    );
    let root = tempfile::tempdir().unwrap();
    let failure = shaders::compile(&root.path().join("absent"), source, 0).unwrap_err();
    assert_eq!(failure.stage, FailureStage::CompilerProcess);
    assert!(failure.diagnostics.is_empty());
}
