//! The 394-probe GLSL ES 3.00 coverage sweep as a regression suite
//! (amendment 9; S3.md, "Coverage sweep"): every probe's translation verdict
//! against WebGL's (`tests/fixtures/probes.json`, written by
//! `engine/crates/pm-render/tools/effects/expectations.ts` from a WebGL2 run
//! on ANGLE Metal). Pixels are compared by pm-render's `effects_webgl` test
//! when the shader corpus capture is present; verdicts are checked always.

use sha2::{Digest, Sha256};

/// Probes whose verdict differs from WebGL's for a documented reason
/// (docs/engine/effects.md, "Coverage sweep").
fn known(id: &str) -> Option<&'static str> {
    let name = id.rsplit('/').next().unwrap_or(id);
    const ALONE: [&str; 8] = [
        "sampler3D-alone",
        "samplerCube-alone",
        "sampler2DArray-alone",
        "samplerCubeShadow-alone",
        "sampler2DArrayShadow-alone",
        "isampler3D-alone",
        "usamplerCube-alone",
        "isampler2DArray-alone",
    ];
    if ALONE.contains(&name) {
        return Some(
            "WebGL samples an unbound non-sampler2D unit as (0, 0, 0, 1); the bind check refuses (fallback runner)",
        );
    }
    if name == "ternary-struct" || name == "ternary-array" {
        return Some(
            "WebGL refuses ?: on structs and arrays; the engine accepts (needs expression types)",
        );
    }
    None
}

#[test]
fn every_probe_gets_webgls_verdict() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/probes.json")).unwrap();
    let show = fixture["show"].as_str().unwrap();
    let tex = fixture["preludeTex"].as_str().unwrap();
    let uv = fixture["preludeUv"].as_str().unwrap();
    let mut unexpected = Vec::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let probes = fixture["probes"].as_array().unwrap();
    assert_eq!(probes.len(), 394);
    for probe in probes {
        let id = probe["id"].as_str().unwrap();
        let frag = probe["template"]
            .as_str()
            .unwrap()
            .replace("@SHOW@", show)
            .replace("@PRELUDE_TEX@", tex)
            .replace("@PRELUDE_UV@", uv);
        let hash: String = Sha256::digest(frag.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            hash,
            probe["sha256"].as_str().unwrap(),
            "{id}: rebuilt program differs"
        );
        let webgl = probe["webgl"].as_str().unwrap();
        let result = pm_glsl::cache::prepare_isolated(&frag, 0, pm_glsl::cache::DEFAULT_DEADLINE)
            .and_then(|p| p.finish().map(|_| ()));
        let verdict = match &result {
            Ok(()) => "translates".to_owned(),
            Err(f) => format!("refused at {}", f.stage.name()),
        };
        *counts
            .entry(format!(
                "WebGL {webgl}, engine {}",
                if result.is_ok() {
                    "translates"
                } else {
                    "refuses"
                }
            ))
            .or_default() += 1;
        let agrees = match (webgl, &result) {
            ("drawn", Ok(())) => true,
            // WebGL refuses at compile or link: the engine refuses in a
            // stage that reproduces WebGL's rules (skipped as today).
            ("refused", Err(f)) => f.stage.webgl_refuses(),
            // WebGL links but its draw fails: the engine refuses at the
            // bind check, or (an integer output on a float target) when
            // wgpu creates the pipeline.
            ("drawError", Err(f)) => f.stage == pm_glsl::FailureStage::Bind,
            ("drawError", Ok(())) => id.ends_with("output-ivec4"),
            _ => false,
        };
        if !agrees && known(id).is_none() {
            unexpected.push(format!("{id}: WebGL {webgl}, engine {verdict}"));
        }
        if agrees && known(id).is_some() {
            unexpected.push(format!(
                "{id}: now agrees with WebGL ({verdict}); remove it from the known list"
            ));
        }
    }
    eprintln!("{counts:#?}");
    assert!(unexpected.is_empty(), "{unexpected:#?}");
}

/// The fuzz target in-process, as a smoke test (`pm-glsl-fuzz` runs the long
/// campaigns in a worker process): deterministic mutants of the probes never
/// panic and every outcome is a verdict.
#[test]
fn mutants_of_the_probes_never_panic() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/probes.json")).unwrap();
    let show = fixture["show"].as_str().unwrap();
    let tex = fixture["preludeTex"].as_str().unwrap();
    let uv = fixture["preludeUv"].as_str().unwrap();
    let seeds: Vec<pm_glsl::fuzz::Seed> = fixture["probes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| pm_glsl::fuzz::Seed {
            id: p["id"].as_str().unwrap().to_owned(),
            frag: p["template"]
                .as_str()
                .unwrap()
                .replace("@SHOW@", show)
                .replace("@PRELUDE_TEX@", tex)
                .replace("@PRELUDE_UV@", uv),
            line_offset: 0,
        })
        .collect();
    let mut tally = std::collections::BTreeMap::<String, usize>::new();
    for index in 0..300 {
        let (source, _, offset) = pm_glsl::fuzz::mutant(&seeds, 5, index);
        let outcome = std::thread::Builder::new()
            .stack_size(pm_glsl::cache::TRANSLATION_STACK)
            .spawn(move || pm_glsl::fuzz::outcome(&source, offset))
            .unwrap()
            .join()
            .expect("the outcome catches panics itself");
        assert_ne!(
            outcome.result, "panic",
            "mutant {index}: {}",
            outcome.detail
        );
        *tally.entry(outcome.result).or_default() += 1;
    }
    eprintln!("{tally:?}");
}
