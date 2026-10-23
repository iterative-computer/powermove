//! Where a bench's heap allocations come from (docs/engine/perfbench.md,
//! "Finding the work"): runs one bench with an allocator that records a
//! backtrace for every Nth allocation on the bench thread and prints the
//! call sites that allocate most, by count.
//!
//!   cargo run --release -p pm-perfbench --example alloc_sites -- <bench> [every N, default 16] [top, default 40]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

struct Sampling;

static ON: AtomicBool = AtomicBool::new(false);
static EVERY: AtomicU64 = AtomicU64::new(16);
static SAMPLES: Mutex<Vec<(std::backtrace::Backtrace, usize)>> = Mutex::new(Vec::new());

thread_local! {
    static IN: Cell<bool> = const { Cell::new(false) };
    static N: Cell<u64> = const { Cell::new(0) };
    static BENCH: Cell<bool> = const { Cell::new(false) };
}

fn sample(size: usize) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let _ = IN.try_with(|guard| {
        if guard.get() || !BENCH.with(Cell::get) {
            return;
        }
        let n = N.with(|n| {
            n.set(n.get() + 1);
            n.get()
        });
        if !n.is_multiple_of(EVERY.load(Ordering::Relaxed)) {
            return;
        }
        guard.set(true);
        let bt = std::backtrace::Backtrace::force_capture();
        if let Ok(mut s) = SAMPLES.lock() {
            s.push((bt, size));
        }
        guard.set(false);
    });
}

// SAFETY: forwards to System; sampling is guarded against reentrancy.
unsafe impl GlobalAlloc for Sampling {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        pm_perf::count_alloc(l.size());
        sample(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        pm_perf::count_alloc(l.size());
        sample(l.size());
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if n > l.size() {
            pm_perf::count_alloc(n);
            sample(n);
        }
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static ALLOC: Sampling = Sampling;

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args.first().expect("a bench name").clone();
    EVERY.store(
        args.get(1).and_then(|v| v.parse().ok()).unwrap_or(16),
        Ordering::Relaxed,
    );
    let top: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(40);
    let ctx = pm_perfbench::benches::Ctx::new();
    let (_, run) = pm_perfbench::benches::all()
        .into_iter()
        .find(|(n, _)| *n == name)
        .expect("a bench");
    BENCH.with(|b| b.set(true));
    ON.store(true, Ordering::Relaxed);
    let r = run(&ctx);
    ON.store(false, Ordering::Relaxed);
    BENCH.with(|b| b.set(false));
    let samples = std::mem::take(&mut *SAMPLES.lock().unwrap());
    eprintln!(
        "{name}: {} samples, counters {}",
        samples.len(),
        r.counters.to_json()
    );
    if std::env::var("PM_ALLOC_RAW").is_ok() {
        eprintln!("{}", samples[samples.len() / 2].0);
    }
    // Group by the first frames inside engine crates (skipping allocator,
    // std and alloc frames).
    let mut by_site: std::collections::HashMap<String, (u64, u64)> = Default::default();
    let mut by_leaf: std::collections::HashMap<String, (u64, u64)> = Default::default();
    for (bt, size) in &samples {
        let text = bt.to_string();
        // Frames inside the engine's crates: "function (crate/src/file.rs:line)".
        let lines: Vec<&str> = text.lines().collect();
        let mut frames: Vec<String> = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            let t = l.trim_start();
            if !t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                continue;
            }
            let name = t.split_once(": ").map(|x| x.1).unwrap_or("").trim();
            let Some(at) = lines
                .get(i + 1)
                .and_then(|n| n.trim_start().strip_prefix("at "))
            else {
                continue;
            };
            let Some(pos) = at.find("/crates/") else {
                continue;
            };
            let file = &at[pos + 8..];
            if file.starts_with("pm-perfbench/") || file.starts_with("pm-perf/") {
                continue;
            }
            let short: String = name
                .rsplit("::")
                .next()
                .unwrap_or(name)
                .chars()
                .take(40)
                .collect();
            frames.push(format!(
                "{short} ({})",
                file.rsplit_once(':').map_or(file, |x| x.0)
            ));
        }
        let site = frames
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" <- ");
        let e = by_site.entry(site).or_default();
        e.0 += 1;
        e.1 += *size as u64;
        let leaf = frames.first().cloned().unwrap_or_default();
        let e = by_leaf.entry(leaf).or_default();
        e.0 += 1;
        e.1 += *size as u64;
    }
    let every = EVERY.load(Ordering::Relaxed);
    for (title, map) in [
        ("by function", by_leaf),
        ("by call chain (3 frames)", by_site),
    ] {
        let mut v: Vec<_> = map.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1.0));
        println!(
            "== {title}: estimated allocations per unit ({} units)",
            r.units
        );
        for (site, (n, bytes)) in v.into_iter().take(top) {
            println!(
                "{:>9.1} allocs {:>12.0} B  {site}",
                (n * every) as f64 / f64::from(r.units),
                (bytes * every) as f64 / f64::from(r.units)
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
