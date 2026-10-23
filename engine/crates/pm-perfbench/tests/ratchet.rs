//! The ratchet (docs/engine/perfbench.md): every bench's counters against
//! `baselines.json`. Fails when any count rises above its baseline.
//!
//! `PM_PERF_RATCHET=update` lowers the baselines that went down and records
//! new benches (never raises one); `PM_PERF_ONLY=<name,...>` runs some
//! benches (the comparison then covers only those).
//!
//! Runs on the main thread (harness = false): the typing bench's CoreText
//! registry uses AppKit's font manager.

#[global_allocator]
static ALLOC: pm_perf::CountingAlloc = pm_perf::CountingAlloc;

#[cfg(target_os = "macos")]
fn main() {
    use pm_perfbench::ratchet::{Baselines, baselines_path, compare, lowered};
    assert!(
        pm_perf::alloc_counting_installed(),
        "the counting allocator is installed"
    );
    let only: Option<Vec<String>> = std::env::var("PM_PERF_ONLY")
        .ok()
        .map(|v| v.split(',').map(str::to_owned).collect());
    let ctx = pm_perfbench::benches::Ctx::new();
    let mut reports = Vec::new();
    for (name, run) in pm_perfbench::benches::all() {
        if only.as_ref().is_some_and(|o| !o.iter().any(|n| n == name)) {
            continue;
        }
        let r = run(&ctx);
        println!(
            "{name}: {} {} wall p50 {:.2} ms (informational) {}",
            r.units,
            r.unit,
            r.wall_at(0.5),
            r.counters.to_json()
        );
        reports.push(r);
    }
    let path = baselines_path();
    let baselines = Baselines::read(&path);
    let outcome = compare(&baselines, &reports);
    for (bench, counter, b, a) in &outcome.improvements {
        println!("improved: {bench} {counter}: {a} < baseline {b}");
    }
    if std::env::var("PM_PERF_RATCHET").is_ok_and(|v| v == "update") {
        let next = lowered(&baselines, &reports);
        next.write(&path);
        println!("baselines updated: {}", path.display());
        let still = compare(&next, &reports).regressions;
        for r in &still {
            println!("still above its baseline (raise it by hand if intended): {r}");
        }
        if !still.is_empty() {
            std::process::exit(1);
        }
        return;
    }
    if !outcome.regressions.is_empty() {
        eprintln!(
            "perf ratchet: {} counts rose above their baselines:",
            outcome.regressions.len()
        );
        for r in &outcome.regressions {
            eprintln!("  {r}");
        }
        eprintln!(
            "Make the change do less work, or raise the baseline by hand in {} with the reason in the commit.",
            path.display()
        );
        std::process::exit(1);
    }
    if !outcome.improvements.is_empty() {
        println!(
            "{} counts are below their baselines: lock them in with PM_PERF_RATCHET=update",
            outcome.improvements.len()
        );
    }
    println!(
        "perf ratchet: {} benches within their baselines",
        reports.len()
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("perf ratchet: macOS only (Metal benches)");
}
