//! The ratchet (docs/engine/perfbench.md): every bench's counters, on the
//! bench thread and on every thread, against `baselines.json`. Fails when
//! any count rises above its baseline, when a bench measures other units
//! than its baseline, or (a whole run) when a bench of the file did not run.
//!
//! `PM_PERF_RATCHET=update` lowers the baselines that went down and records
//! new benches (never raises one, never changes units, refuses while a bench
//! of the file did not run, and runs only with the whole set);
//! `PM_PERF_ONLY=<name,...>` runs some benches (the comparison then covers
//! only those).
//!
//! Runs on the main thread (harness = false): the typing bench's CoreText
//! registry uses AppKit's font manager.

#[global_allocator]
static ALLOC: pm_perf::CountingAlloc = pm_perf::CountingAlloc;

#[cfg(target_os = "macos")]
fn main() {
    use pm_perfbench::ratchet::{Baselines, baselines_path, compare, lowered};
    const {
        assert!(
            pm_perf::ENABLED,
            "pm-perf is built with its measurement-only `noop` feature: nothing is counted"
        )
    };
    assert!(
        pm_perf::alloc_counting_installed(),
        "the counting allocator is installed"
    );
    let only: Option<Vec<String>> = std::env::var("PM_PERF_ONLY")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| v.split(',').map(str::to_owned).collect());
    let benches = pm_perfbench::benches::all();
    for name in only.iter().flatten() {
        assert!(
            benches.iter().any(|(n, _)| n == name),
            "PM_PERF_ONLY names {name}, which is not a bench"
        );
    }
    let whole_set = only.is_none();
    let ctx = pm_perfbench::benches::Ctx::new();
    let mut reports = Vec::new();
    for (name, run) in benches {
        if only.as_ref().is_some_and(|o| !o.iter().any(|n| n == name)) {
            continue;
        }
        let r = run(&ctx);
        println!(
            "{name}: {} {} wall p50 {:.2} ms (informational) thread {} all threads {}",
            r.units,
            r.unit,
            r.wall_at(0.5),
            r.counters.to_json(),
            r.global.to_json()
        );
        reports.push(r);
    }
    let path = baselines_path();
    let baselines = Baselines::read(&path);
    let outcome = compare(&baselines, &reports, whole_set);
    for (bench, scope, counter, b, a) in &outcome.improvements {
        println!(
            "improved: {bench} {counter} ({}): {a} < baseline {b}",
            scope.key()
        );
    }
    if std::env::var("PM_PERF_RATCHET").is_ok_and(|v| v == "update") {
        // A bench alone can count less than in the whole set (state warmed by
        // the benches before it), so only whole runs lower baselines.
        if !whole_set {
            eprintln!(
                "perf ratchet: PM_PERF_RATCHET=update runs the whole set (unset PM_PERF_ONLY)"
            );
            std::process::exit(1);
        }
        let next = match lowered(&baselines, &reports, whole_set) {
            Ok(next) => next,
            Err(e) => {
                eprintln!("perf ratchet: {e}");
                std::process::exit(1);
            }
        };
        for r in &reports {
            if !baselines.benches.contains_key(&r.name) {
                println!(
                    "recorded a new bench: {} (review its counts in the diff)",
                    r.name
                );
            }
        }
        next.write(&path);
        println!("baselines updated: {}", path.display());
        let still = compare(&next, &reports, whole_set).regressions;
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
            "perf ratchet: {} failures (counts above their baselines, changed units, benches that did not run):",
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
