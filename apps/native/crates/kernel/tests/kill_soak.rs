//! The same repeatable child-process harness is available as a long-running binary.
#[test]
fn randomized_sigkill_transactions_folding_autosave_discard() {
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_pm-journal-soak"))
        .args(["--rounds", "18", "--seed", "71"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&result.stdout)
            .matches("PASS round=")
            .count(),
        18
    );
}
