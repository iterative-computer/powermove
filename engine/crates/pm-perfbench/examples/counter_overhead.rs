//! What the always-on counters cost: the viewer benches as the app runs
//! them (no counting allocator, process-wide totals off), for an A/B against
//! a build with `pm-perf/noop`, which compiles every add out
//! (docs/engine/perfbench.md, "Overhead").
//!
//!   cargo build --release -p pm-perfbench --example counter_overhead
//!   cargo build --release -p pm-perfbench --example counter_overhead --features pm-perf/noop --target-dir target/noop
//!   counter_overhead [bench ...] [--repeat N]     (prints one JSON line per run)
//!   counter_overhead --micro                      (ns per add, and adds per unit of each bench)
//!
//! Run the two binaries alternately: wall-clock on this machine moves with
//! its load, so only interleaved pairs compare.

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut names = Vec::new();
    let mut repeat = 1;
    let mut i = 0;
    if args.iter().any(|a| a == "--micro") {
        micro();
        return;
    }
    while i < args.len() {
        match args[i].as_str() {
            "--repeat" => {
                repeat = args[i + 1].parse().expect("--repeat N");
                i += 1;
            }
            n => names.push(n.to_owned()),
        }
        i += 1;
    }
    if names.is_empty() {
        names = vec!["playback_500".into(), "drag_500".into()];
    }
    let ctx = pm_perfbench::benches::Ctx::new();
    // As in the app: each add touches its thread's cell only.
    pm_perf::count_globally(false);
    for round in 0..repeat {
        for (name, run) in pm_perfbench::benches::all() {
            if !names.iter().any(|n| n == name) {
                continue;
            }
            let r = run(&ctx);
            println!(
                "{}",
                serde_json::json!({
                    "bench": name,
                    "round": round,
                    "counting": pm_perf::ENABLED,
                    "units": r.units,
                    "wall_ms": { "p50": r.wall_at(0.5), "p95": r.wall_at(0.95), "total": r.wall_total() },
                    "draws": r.counters.get(pm_perf::Counter::DrawCalls),
                })
            );
        }
    }
}

/// The cost of one add, timed over many (thread-local only, as in the app,
/// and with the process-wide totals on, as in the benches), and how many adds
/// each viewer bench makes per unit (every add bumps one count counter;
/// byte counters ride along with one).
#[cfg(target_os = "macos")]
fn micro() {
    use pm_perf::Counter;
    use std::time::Instant;
    const N: u64 = 50_000_000;
    let time = |global: bool| {
        pm_perf::count_globally(global);
        let start = Instant::now();
        for i in 0..N {
            pm_perf::add(
                std::hint::black_box(Counter::DrawCalls),
                std::hint::black_box(i & 1),
            );
        }
        start.elapsed().as_secs_f64() * 1e9 / N as f64
    };
    let thread_ns = (0..5).map(|_| time(false)).fold(f64::INFINITY, f64::min);
    let global_ns = (0..5).map(|_| time(true)).fold(f64::INFINITY, f64::min);
    pm_perf::count_globally(false);
    let ctx = pm_perfbench::benches::Ctx::new();
    pm_perf::count_globally(false);
    for (name, run) in pm_perfbench::benches::all() {
        if !["playback_500", "drag_500", "inspector_edit_500"].contains(&name) {
            continue;
        }
        let r = run(&ctx);
        let adds: u64 = r
            .counters
            .iter()
            .filter(|(c, _)| !c.name().ends_with("_bytes"))
            .map(|(_, v)| v)
            .sum();
        let per_unit = adds as f64 / f64::from(r.units);
        println!(
            "{}",
            serde_json::json!({
                "bench": name,
                "adds_per_unit": per_unit.round(),
                "ns_per_add_thread": thread_ns,
                "ns_per_add_global": global_ns,
                "us_per_unit_thread": per_unit * thread_ns / 1e3,
                "wall_p50_ms": r.wall_at(0.5),
            })
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
