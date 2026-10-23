//! Runs the benches and prints each report as JSON (counters plus
//! informational wall-clock), for measurement and correlation runs
//! (docs/engine/perfbench.md).
//!
//!   cargo run --release -p pm-perfbench --example perfbench -- [name ...] [--repeat N] [--out file.jsonl]

#[global_allocator]
static ALLOC: pm_perf::CountingAlloc = pm_perf::CountingAlloc;

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut names = Vec::new();
    let mut repeat = 1;
    let mut out: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--repeat" => {
                repeat = args[i + 1].parse().expect("--repeat N");
                i += 1;
            }
            "--out" => {
                out = Some(args[i + 1].clone());
                i += 1;
            }
            n => names.push(n.to_owned()),
        }
        i += 1;
    }
    let ctx = pm_perfbench::benches::Ctx::new();
    let mut lines = Vec::new();
    for round in 0..repeat {
        for (name, run) in pm_perfbench::benches::all() {
            if !names.is_empty() && !names.iter().any(|n| n == name) {
                continue;
            }
            let r = run(&ctx);
            let mut j = r.to_json();
            j["round"] = round.into();
            j["load"] = load().into();
            let line = j.to_string();
            println!("{line}");
            lines.push(line);
        }
    }
    if let Some(path) = out {
        std::fs::write(path, lines.join("\n") + "\n").expect("write the report");
    }
}

/// The 1-minute load average (the machine is shared; every report records it).
#[cfg(target_os = "macos")]
fn load() -> f64 {
    let mut l = [0f64; 3];
    // SAFETY: getloadavg writes at most 3 doubles.
    let n = unsafe { getloadavg(l.as_mut_ptr(), 3) };
    if n > 0 { l[0] } else { -1.0 }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn getloadavg(loadavg: *mut f64, nelem: i32) -> i32;
}

#[cfg(not(target_os = "macos"))]
fn main() {}
