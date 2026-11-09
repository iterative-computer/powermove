#[test]
fn standalone_entry_defaults_to_main_and_retains_explicit_legacy_checks() {
    let bin = env!("CARGO_BIN_EXE_pm-e2e-triage");
    assert_eq!(
        std::process::Command::new(bin)
            .arg("--check")
            .output()
            .unwrap()
            .status
            .code(),
        Some(64)
    );
    let output = std::process::Command::new(bin)
        .args(["--check", "MAIN.MCP.list_projects.catalog"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["reference"], pm_acceptance::main_reference::REFERENCE);
    let output = std::process::Command::new(bin)
        .args([
            "--reference",
            "v1.2.0",
            "--check",
            "selection_preserves_redo",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["reference"], pm_acceptance::e2e::inventory::REFERENCE);
}
