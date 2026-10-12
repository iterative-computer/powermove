//! Scores replayed outputs against the three.js references with the section 2 3D bar.
//!
//!   s7-compare <ref-root> <test-root> [scene ...]
use anyhow::{Context, Result};
use s7_replay::compare::score_scene;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ref_root = PathBuf::from(args.first().context("ref root")?);
    let test_root = PathBuf::from(args.get(1).context("test root")?);
    let mut scenes: Vec<String> = args[2..].to_vec();
    if scenes.is_empty() {
        let mut all: Vec<String> = std::fs::read_dir(&test_root)?.filter_map(|e| e.ok()).filter(|e| e.path().is_dir() && ref_root.join(e.file_name()).join("info.json").exists()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        all.sort();
        scenes = all;
    }
    let mut report = Vec::new();
    let (mut outputs, mut passed) = (0, 0);
    println!("{:<26} {:<16} {:>9} {:>5} {:>8} {:>9} {:>5}", "scene", "output", "<=2CV %", "max", "max out", "min SSIM", "pass");
    for scene in &scenes {
        match score_scene(&ref_root.join(scene), &test_root.join(scene), Some(&test_root.join(scene).join("diff"))) {
            Ok(scores) => {
                for s in &scores {
                    outputs += 1;
                    if s.pass {
                        passed += 1;
                    }
                    println!("{:<26} {:<16} {:>9.3} {:>5} {:>8} {:>9} {:>5}", scene, s.output, s.fraction_within_2 * 100.0, s.max_cv, s.max_cv_outside_edges, s.min_object_ssim.map(|v| format!("{v:.4}")).unwrap_or("-".into()), if s.pass { "PASS" } else { "FAIL" });
                }
                report.push(serde_json::json!({ "scene": scene, "outputs": scores }));
            }
            Err(e) => {
                println!("{scene:<26} ERROR {e:#}");
                report.push(serde_json::json!({ "scene": scene, "error": format!("{e:#}") }));
            }
        }
    }
    println!("{passed}/{outputs} outputs pass");
    std::fs::write(test_root.join("compare.json"), serde_json::to_string_pretty(&report)?)?;
    Ok(())
}
