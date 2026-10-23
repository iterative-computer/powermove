//! Runs the benches and prints each report as JSON (counters plus
//! informational wall-clock), for measurement and correlation runs
//! (docs/engine/perfbench.md).
//!
//!   cargo run --release -p pm-perfbench --example perfbench -- [name ...] [--repeat N] [--out file.jsonl]
//!   cargo run --release -p pm-perfbench --example perfbench -- --correlate [--repeat N] [--out file.jsonl]
//!
//! `--correlate` runs the correlation set (`benches::correlation`) N times,
//! interleaved, and prints, for every counter, how it tracks wall-clock
//! across the set (Pearson r and Spearman rho of counts per unit against the
//! median wall-clock per unit) and in paired runs (configurations that
//! differ by one knob: does wall-clock move the way the counter moved?).

#[global_allocator]
static ALLOC: pm_perf::CountingAlloc = pm_perf::CountingAlloc;

#[cfg(target_os = "macos")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut names = Vec::new();
    let mut repeat = 1;
    let mut out: Option<String> = None;
    let mut correlate = false;
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
            "--correlate" => correlate = true,
            n => names.push(n.to_owned()),
        }
        i += 1;
    }
    let ctx = pm_perfbench::benches::Ctx::new();
    if correlate {
        correlation(&ctx, repeat, out.as_deref());
        return;
    }
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

/// The correlation set, `repeat` rounds interleaved, and its analysis.
#[cfg(target_os = "macos")]
fn correlation(ctx: &pm_perfbench::benches::Ctx, repeat: usize, out: Option<&str>) {
    use pm_perf::Counter;
    use std::collections::BTreeMap;
    let set = pm_perfbench::benches::correlation();
    // name -> (counts per unit, wall p50 per round)
    let mut runs: BTreeMap<String, (Vec<f64>, Vec<f64>, Vec<f64>)> = BTreeMap::new();
    let mut lines = Vec::new();
    for round in 0..repeat.max(1) {
        for (name, run) in &set {
            let r = run(ctx);
            let per_unit: Vec<f64> = Counter::ALL
                .iter()
                .map(|&c| r.counters.get(c) as f64 / f64::from(r.units.max(1)))
                .collect();
            let e = runs
                .entry(name.clone())
                .or_insert_with(|| (per_unit.clone(), vec![], vec![]));
            e.1.push(r.wall_at(0.5));
            e.2.push(load());
            let mut j = r.to_json();
            j["round"] = round.into();
            j["load"] = load().into();
            lines.push(j.to_string());
            eprintln!(
                "{name} round {round}: wall p50 {:.3} ms, load {:.1}",
                r.wall_at(0.5),
                load()
            );
        }
    }
    let median = |xs: &[f64]| {
        let mut v = xs.to_vec();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let names: Vec<&String> = runs.keys().collect();
    let wall: Vec<f64> = names.iter().map(|n| median(&runs[*n].1)).collect();
    // Paired families: same workload, one knob apart.
    let family = |n: &str| -> String {
        let parts: Vec<&str> = n.split('_').collect();
        match parts[0] {
            "drag" => format!("drag_{}", parts.get(2).unwrap_or(&"")),
            p => p.to_owned(),
        }
    };
    let mut table = Vec::new();
    for (ci, &c) in Counter::ALL.iter().enumerate() {
        let xs: Vec<f64> = names.iter().map(|n| runs[*n].0[ci]).collect();
        if xs.iter().all(|&x| x == 0.0) {
            continue;
        }
        let r = pearson(&xs, &wall);
        let rho = pearson(&ranks(&xs), &ranks(&wall));
        // Paired: within a family, every pair whose counts differ by > 5%.
        let (mut agree, mut pairs) = (0, 0);
        for a in 0..names.len() {
            for b in a + 1..names.len() {
                if family(names[a]) != family(names[b])
                    && !(names[a].starts_with("drag_") && names[b].starts_with("drag_"))
                {
                    continue;
                }
                let (xa, xb) = (xs[a], xs[b]);
                if (xa - xb).abs() <= 0.05 * xa.max(xb) {
                    continue;
                }
                pairs += 1;
                if (xa - xb).signum() == (wall[a] - wall[b]).signum() {
                    agree += 1;
                }
            }
        }
        table.push(serde_json::json!({
            "counter": c.name(), "pearson": r, "spearman": rho,
            "paired_agree": agree, "paired": pairs,
        }));
        println!(
            "{:<32} r {:>6.3}  rho {:>6.3}  paired {:>2}/{:<2}",
            c.name(),
            r,
            rho,
            agree,
            pairs
        );
    }
    let configs: Vec<serde_json::Value> = names
        .iter()
        .zip(&wall)
        .map(|(n, w)| serde_json::json!({ "name": n, "wall_p50_ms": w, "walls": runs[*n].1, "loads": runs[*n].2 }))
        .collect();
    let summary = serde_json::json!({ "correlation": table, "configs": configs });
    println!("{summary}");
    if let Some(path) = out {
        lines.push(summary.to_string());
        std::fs::write(path, lines.join("\n") + "\n").expect("write the report");
    }
}

#[cfg(target_os = "macos")]
fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    if sxx == 0.0 || syy == 0.0 {
        0.0
    } else {
        sxy / (sxx * syy).sqrt()
    }
}

#[cfg(target_os = "macos")]
fn ranks(x: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&a, &b| x[a].total_cmp(&x[b]));
    let mut r = vec![0.0; x.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0;
        for k in i..=j {
            r[idx[k]] = avg;
        }
        i = j + 1;
    }
    r
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
