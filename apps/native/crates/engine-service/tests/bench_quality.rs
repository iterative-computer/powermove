//! The bench's scrub settle waits for the quality it was asked to run at.

#[test]
fn a_half_quality_bench_settles_at_half() {
    let report = tempfile::NamedTempFile::new().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_pm-engine-service"))
        .args([
            "--bench",
            "--seconds=1",
            "--quality=half",
            &format!("--report={}", report.path().display()),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report.path()).unwrap()).unwrap();
    assert!(
        report["scrub_ms"]["settle_exact"].as_f64().is_some(),
        "{report}"
    );
    assert_eq!(report["lowest_effective_quality"], 0.5, "{report}");
}
