//! Batch summaries with 90 % intervals, and DESIGN.md 8's bar for "Aero is
//! better": a higher success rate than each other harness on the same model
//! with non-overlapping 90 % intervals, and median wall time and tokens no
//! worse.
use crate::record::RunRecord;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// z for a two-sided 90 % interval.
pub const Z90: f64 = 1.644_853_626_951_472_2;

/// Wilson score interval for k successes in n trials.
pub fn wilson(k: u32, n: u32, z: f64) -> Interval {
    if n == 0 {
        return Interval {
            estimate: 0.0,
            lo: 0.0,
            hi: 1.0,
        };
    }
    let (all, none) = (k == n, k == 0);
    let (k, n) = (k as f64, n as f64);
    let p = k / n;
    let z2 = z * z;
    let centre = (p + z2 / (2.0 * n)) / (1.0 + z2 / n);
    let half = z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / (1.0 + z2 / n);
    // The bounds are exactly 0 and 1 at the extremes (no rounding residue).
    Interval {
        estimate: p,
        lo: if none { 0.0 } else { (centre - half).max(0.0) },
        hi: if all { 1.0 } else { (centre + half).min(1.0) },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub estimate: f64,
    pub lo: f64,
    pub hi: f64,
}

/// Median with a distribution-free confidence interval from order statistics:
/// [x(k), x(n−k+1)] with the largest k such that P(Bin(n, ½) < k) ≤ (1−level)/2.
/// When even the sample range cannot reach the level (n ≤ 4 at 90 %), the
/// interval is the sample range.
pub fn median_interval(samples: &[f64], level: f64) -> Option<Interval> {
    if samples.is_empty() {
        return None;
    }
    let mut s = samples.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    let median = if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    };
    let alpha = (1.0 - level) / 2.0;
    // cdf[j] = P(Bin(n, ½) ≤ j)
    let mut k = 1usize;
    let mut cdf = 0.0;
    let mut coeff = 1.0f64; // C(n, j)
    for j in 0..n {
        if j > 0 {
            coeff = coeff * (n - j + 1) as f64 / j as f64;
        }
        cdf += coeff / 2f64.powi(n as i32);
        // Coverage of [x(k), x(n−k+1)] is 1 − 2·P(Bin ≤ k−1): k = j+1 is
        // allowed while that tail is ≤ alpha.
        if cdf <= alpha {
            k = j + 1;
        } else {
            break;
        }
    }
    let k = k.min(n.div_ceil(2));
    Some(Interval {
        estimate: median,
        lo: s[k - 1],
        hi: s[n - k],
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub harness: String,
    pub model: String,
    pub effort: String,
    /// `all` for the pooled group, else the task id.
    pub task: String,
    pub runs: u32,
    pub successes: u32,
    pub success: Interval,
    pub renders_correct: Option<Interval>,
    pub unavailable: u32,
    pub wall_ms: Option<Interval>,
    pub tokens_total: Option<Interval>,
    pub tokens_input: Option<Interval>,
    pub tokens_cached: Option<Interval>,
    pub tokens_output: Option<Interval>,
    pub tool_calls_mean: f64,
    pub retries_mean: f64,
    pub gate_continuations_mean: Option<f64>,
    pub needs_fix: Option<Interval>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub format: String,
    pub suite_version: u32,
    pub level: f64,
    pub groups: Vec<Group>,
    pub bar: Vec<BarVerdict>,
    /// Runs left out of the summary and why (another suite version, errors).
    pub excluded: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BarVerdict {
    pub model: String,
    pub effort: String,
    pub against: String,
    pub better: bool,
    pub reasons: Vec<String>,
}

fn group(task: &str, runs: &[&RunRecord]) -> Group {
    let r0 = runs[0];
    let n = runs.len() as u32;
    let k = runs.iter().filter(|r| r.success).count() as u32;
    let rc: Vec<bool> = runs.iter().filter_map(|r| r.renders_correct).collect();
    let nf: Vec<bool> = runs.iter().filter_map(|r| r.needs_fix).collect();
    let mi = |f: &dyn Fn(&RunRecord) -> f64| {
        median_interval(&runs.iter().map(|r| f(r)).collect::<Vec<_>>(), 0.90)
    };
    let mean = |f: &dyn Fn(&RunRecord) -> f64| runs.iter().map(|r| f(r)).sum::<f64>() / n as f64;
    let gates: Vec<f64> = runs
        .iter()
        .filter_map(|r| r.gate_continuations.map(f64::from))
        .collect();
    Group {
        harness: r0.harness.clone(),
        model: r0.model.clone(),
        effort: r0.effort.clone(),
        task: task.into(),
        runs: n,
        successes: k,
        success: wilson(k, n, Z90),
        renders_correct: (!rc.is_empty()).then(|| {
            wilson(
                rc.iter().filter(|b| **b).count() as u32,
                rc.len() as u32,
                Z90,
            )
        }),
        unavailable: runs.iter().filter(|r| r.unavailable).count() as u32,
        wall_ms: mi(&|r| r.wall_ms as f64),
        tokens_total: mi(&|r| r.tokens.total() as f64),
        tokens_input: mi(&|r| r.tokens.input as f64),
        tokens_cached: mi(&|r| r.tokens.cached as f64),
        tokens_output: mi(&|r| r.tokens.output as f64),
        tool_calls_mean: mean(&|r| r.tool_calls as f64),
        retries_mean: mean(&|r| r.retries as f64),
        gate_continuations_mean: (!gates.is_empty())
            .then(|| gates.iter().sum::<f64>() / gates.len() as f64),
        needs_fix: (!nf.is_empty()).then(|| {
            wilson(
                nf.iter().filter(|b| **b).count() as u32,
                nf.len() as u32,
                Z90,
            )
        }),
    }
}

/// Summarise runs of one suite version; others are excluded and listed.
pub fn summarize(runs: &[RunRecord], suite_version: u32) -> Summary {
    let mut excluded = Vec::new();
    let mut keep: Vec<&RunRecord> = Vec::new();
    for r in runs {
        if r.suite_version != suite_version {
            excluded.push(format!(
                "{} {} rep {}: suite version {}",
                r.task, r.harness, r.rep, r.suite_version
            ));
        } else {
            keep.push(r);
        }
    }
    let mut by: BTreeMap<(String, String, String, String), Vec<&RunRecord>> = BTreeMap::new();
    for r in &keep {
        let key = (r.harness.clone(), r.model.clone(), r.effort.clone());
        by.entry((key.0.clone(), key.1.clone(), key.2.clone(), "all".into()))
            .or_default()
            .push(r);
        by.entry((key.0, key.1, key.2, r.task.clone()))
            .or_default()
            .push(r);
    }
    let groups: Vec<Group> = by.iter().map(|(k, v)| group(&k.3, v)).collect();
    let bar = bar(&groups);
    Summary {
        format: "aero-eval-summary".into(),
        suite_version,
        level: 0.90,
        groups,
        bar,
        excluded,
    }
}

/// Aero against every other harness on the same model and effort, pooled.
pub fn bar(groups: &[Group]) -> Vec<BarVerdict> {
    let pooled: Vec<&Group> = groups.iter().filter(|g| g.task == "all").collect();
    let mut out = Vec::new();
    for a in pooled.iter().filter(|g| g.harness == "aero") {
        for o in pooled
            .iter()
            .filter(|g| g.harness != "aero" && g.model == a.model && g.effort == a.effort)
        {
            let mut reasons = Vec::new();
            let higher = a.success.lo > o.success.hi;
            reasons.push(format!(
                "success {:.0}% [{:.0}, {:.0}] vs {:.0}% [{:.0}, {:.0}]: {}",
                a.success.estimate * 100.0,
                a.success.lo * 100.0,
                a.success.hi * 100.0,
                o.success.estimate * 100.0,
                o.success.lo * 100.0,
                o.success.hi * 100.0,
                if higher {
                    "higher, intervals apart"
                } else {
                    "not separated"
                }
            ));
            let no_worse = |name: &str, x: &Option<Interval>, y: &Option<Interval>| match (x, y) {
                (Some(x), Some(y)) => {
                    let ok = x.estimate <= y.estimate;
                    (
                        ok,
                        format!(
                            "median {name} {:.0} vs {:.0}: {}",
                            x.estimate,
                            y.estimate,
                            if ok { "no worse" } else { "worse" }
                        ),
                    )
                }
                _ => (false, format!("median {name}: no samples")),
            };
            let (w_ok, w) = no_worse("wall ms", &a.wall_ms, &o.wall_ms);
            let (t_ok, t) = no_worse("tokens", &a.tokens_total, &o.tokens_total);
            reasons.push(w);
            reasons.push(t);
            out.push(BarVerdict {
                model: a.model.clone(),
                effort: a.effort.clone(),
                against: o.harness.clone(),
                better: higher && w_ok && t_ok,
                reasons,
            });
        }
    }
    out
}

/// Plain-text table for the terminal.
pub fn render(s: &Summary) -> String {
    let mut out = format!(
        "suite v{} — {:.0}% intervals\n",
        s.suite_version,
        s.level * 100.0
    );
    out.push_str("harness       model           effort  task                          runs  success            median wall   median tokens\n");
    for g in &s.groups {
        out.push_str(&format!(
            "{:<13} {:<15} {:<7} {:<29} {:>4}  {:>3.0}% [{:>3.0},{:>3.0}]  {:>10}  {:>13}\n",
            g.harness,
            g.model,
            g.effort,
            g.task,
            g.runs,
            g.success.estimate * 100.0,
            g.success.lo * 100.0,
            g.success.hi * 100.0,
            g.wall_ms
                .map(|i| format!("{:.1}s", i.estimate / 1000.0))
                .unwrap_or_default(),
            g.tokens_total
                .map(|i| format!("{:.0}", i.estimate))
                .unwrap_or_default(),
        ));
    }
    for b in &s.bar {
        out.push_str(&format!(
            "aero vs {} on {} {}: {}\n  {}\n",
            b.against,
            b.model,
            b.effort,
            if b.better {
                "BETTER"
            } else {
                "not shown better"
            },
            b.reasons.join("\n  ")
        ));
    }
    for e in &s.excluded {
        out.push_str(&format!("excluded: {e}\n"));
    }
    out
}
