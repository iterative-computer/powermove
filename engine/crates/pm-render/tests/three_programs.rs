//! Program generation against three.js r180 (amendment 11): the sources
//! `pm_render::three::program` assembles equal, byte for byte, the sources
//! three.js gave WebGL for the same material and lights
//! (`tools/three-ref/run.ts` records them and writes the hashes). On a
//! mismatch, the first differing line is reported against the recorded
//! source when a reference capture is on this machine.

mod support;

use pm_render::three::program::{sources, standard_material};
use sha2::{Digest, Sha256};

fn sha(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn first_difference(ours: &str, theirs: &str) -> String {
    for (i, (a, b)) in ours.lines().zip(theirs.lines()).enumerate() {
        if a != b {
            return format!("line {}: ours {a:?}, three.js {b:?}", i + 1);
        }
    }
    format!(
        "{} lines against {} lines",
        ours.lines().count(),
        theirs.lines().count()
    )
}

#[test]
fn standard_programs_equal_three_js() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/three-programs.json"
    ))
    .unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let programs = doc["programs"].as_array().unwrap();
    assert!(!programs.is_empty());
    let capture = support::newest_capture("three-");
    let mut failures = Vec::new();
    for p in programs {
        let case = p["case"].as_str().unwrap();
        let params = standard_material(
            "Surface",
            p["doubleSided"].as_bool().unwrap(),
            p["hemisphereLights"].as_u64().unwrap() as u32,
            p["directionalLights"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let (vertex, fragment) = sources(&params).unwrap();
        for (stage, ours, expected) in [
            ("vertex", &vertex, &p["vertex"]),
            ("fragment", &fragment, &p["fragment"]),
        ] {
            if sha(ours) == expected["sha256"].as_str().unwrap() {
                continue;
            }
            if let Some(dir) = std::env::var_os("PM_THREE_DUMP") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("{case}.{stage}.glsl")), ours).unwrap();
            }
            let detail = capture
                .as_ref()
                .and_then(|dir| std::fs::read_to_string(dir.join(case).join("record.json")).ok())
                .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
                .and_then(|r| r["programs"][0][stage].as_str().map(|t| first_difference(ours, t)))
                .unwrap_or_else(|| "no reference capture on this machine".into());
            failures.push(format!(
                "{case} {stage}: {} bytes against {} ({detail})",
                ours.len(),
                expected["bytes"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
