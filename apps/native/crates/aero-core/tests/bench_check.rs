//! The budget checker never reports green without data (Sol's m2 item 8):
//! it runs `tools/aero-bench-check.py` on synthetic reports. An absent
//! baseline, a run with no rows and a non-finite reading are errors; the
//! two-run rule holds for budgets as for instructions, so reversing the runs
//! never changes the verdict; resume compares against fx on the same journal.

use std::path::{Path, PathBuf};
use std::process::Command;

const ROWS: [&str; 8] = [
    "harness_start",
    "session_open",
    "resume_200",
    "close",
    "prompt_to_request",
    "body_to_first_text",
    "tool_call_to_host",
    "result_to_request",
];
const JOURNAL: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn checker() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/aero-bench-check.py")
}

fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aero-bench-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A report whose every reading is `wall` ms and 1.0 M instructions, with
/// `idle_bytes` resident per idle session.
fn report(wall: &str, idle_bytes: u64) -> String {
    let rows: Vec<String> = ROWS
        .iter()
        .map(|row| {
            format!(
                r#""{row}":{{"samples":10,"wall_ms":{{"p50":{wall},"p90":{wall},"p99":{wall}}},"minstr":{{"p50":1.0,"p90":1.0,"p99":1.0}},"budget_ms":{{"p50":{},"p90":null,"p99":null}}}}"#,
                if *row == "resume_200" { "0" } else { "0.1" }
            )
        })
        .collect();
    format!(
        r#"{{"schema":1,"rows":{{{}}},"copies":{{"rust_allocations_per_text_delta":0,"rust_allocations_per_tool_answer":0}},"memory":{{"resident_per_session_idle_bytes":{idle_bytes},"resident_per_session_after_prompt_bytes":1000,"budget_idle_bytes":358400,"budget_after_prompt_bytes":819200}},"threads":{{"before":3,"after_close":3}},"resume_journal_sha256":"{JOURNAL}"}}"#,
        rows.join(",")
    )
}

fn baseline() -> String {
    let rows: Vec<String> = ROWS.iter().map(|row| format!(r#""{row}":1.0"#)).collect();
    format!(
        r#"{{"schema":1,"runs":2,"rows":{{{}}},"resume_journal_sha256":"{JOURNAL}"}}"#,
        rows.join(",")
    )
}

/// The checker's exit code on `args`, with `baseline` as the baseline file
/// and fx's restore measured on the same journal.
fn run(dir: &Path, baseline: &Path, args: &[&Path], idle: bool) -> i32 {
    let fx = dir.join("fx.json");
    std::fs::write(
        &fx,
        format!(r#"{{"journal_sha256":"{JOURNAL}","journal_restore_create_ms":{{"p50":5.0}}}}"#),
    )
    .unwrap();
    let mut command = Command::new("python3");
    command
        .arg(checker())
        .env("AERO_BENCH_BASELINE", baseline)
        .env("AERO_FX_RESUME", &fx);
    if idle {
        command.arg("--idle");
    }
    let output = command.args(args).output().expect("run python3");
    output.status.code().unwrap_or(-1)
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn the_checker_never_passes_without_data_and_its_verdict_ignores_run_order() {
    let dir = scratch();
    let base = write(&dir, "baseline.json", &baseline());
    let ok = write(&dir, "ok.json", &report("0.05", 1000));
    assert_eq!(run(&dir, &base, &[&ok], true), 0, "a good run passes");

    // An absent baseline is an error, not a pass.
    let absent = dir.join("absent.json");
    assert_eq!(
        run(&dir, &absent, &[&ok], false),
        1,
        "no baseline passed the check"
    );
    // A run with no rows.
    let empty = write(&dir, "empty.json", r#"{"schema":1,"rows":{}}"#);
    assert_eq!(
        run(&dir, &absent, &[&empty], false),
        1,
        "no rows, no baseline passed"
    );
    assert_eq!(run(&dir, &base, &[&empty], false), 1, "no rows passed");
    // A non-finite reading (Python's json reads NaN and Infinity).
    for broken in ["NaN", "Infinity"] {
        let bad = write(&dir, "bad.json", &report(broken, 1000));
        assert_eq!(
            run(&dir, &base, &[&bad], true),
            1,
            "a {broken} reading passed"
        );
    }

    // The two-run rule for budgets: one run over the idle memory budget and
    // one under give the same verdict in either order.
    let fat = write(&dir, "fat.json", &report("0.05", 400_000));
    let forward = run(&dir, &base, &[&ok, &fat], true);
    let reversed = run(&dir, &base, &[&fat, &ok], true);
    assert_eq!(
        forward, reversed,
        "the order of the runs changed the verdict"
    );
    assert_eq!(forward, 0, "a miss that did not reproduce failed");
    assert_eq!(
        run(&dir, &base, &[&fat, &fat], true),
        1,
        "a reproduced miss passed"
    );

    // The checker's own cases.
    let status = Command::new("python3")
        .arg(checker())
        .arg("--self-test")
        .status()
        .expect("run python3");
    assert!(status.success(), "the checker's self-test failed");
    let _ = std::fs::remove_dir_all(&dir);
}
