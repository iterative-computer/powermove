//! Batch summaries with 90 % intervals, and DESIGN.md 8's bar for "Aero is
//! better": on the same model and effort, a higher success rate than every
//! other harness with non-overlapping 90 % intervals, and median wall time
//! and tokens no worse. The bar only compares like with like: live runs of
//! the full harness (no ablation, no replay), inside one cohort both
//! harnesses ran in as one interleaved batch, on a balanced sample (the same
//! tasks, at least [`MIN_RUNS_PER_TASK`] repeats per task run on both sides,
//! the same number from every task), and observed cost telemetry on every
//! compared run.
use crate::record::RunRecord;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// z for a two-sided 90 % interval.
pub const Z90: f64 = 1.644_853_626_951_472_2;
/// DESIGN.md 8: at least five runs per task per harness.
pub const MIN_RUNS_PER_TASK: usize = 5;

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

/// A median with its distribution-free interval. The bounds are absent when
/// too few samples exist for any order-statistic interval to reach the level
/// (n ≤ 4 at 90 %); `coverage` is the interval's attained coverage.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MedianInterval {
    pub estimate: f64,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub coverage: Option<f64>,
}

/// P(Bin(n, ½) ≤ j) for j = 0..n, summed in log space so it stays exact to
/// rounding for any n (the coefficients never overflow).
fn binomial_half_cdf(n: usize) -> Vec<f64> {
    let ln_half_n = -(n as f64) * std::f64::consts::LN_2;
    let mut ln_pmf = ln_half_n;
    let mut cdf = Vec::with_capacity(n + 1);
    let mut sum = 0.0;
    for j in 0..=n {
        if j > 0 {
            ln_pmf += ((n - j + 1) as f64).ln() - (j as f64).ln();
        }
        sum += ln_pmf.exp();
        cdf.push(sum.min(1.0));
    }
    cdf
}

/// Median with a distribution-free interval from order statistics:
/// [x(k), x(n−k+1)] with the largest k whose coverage 1 − 2·P(Bin(n, ½) ≤ k−1)
/// is at least `level`.
pub fn median_interval(samples: &[f64], level: f64) -> Option<MedianInterval> {
    if samples.is_empty() {
        return None;
    }
    let mut s = samples.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    let estimate = if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    };
    let cdf = binomial_half_cdf(n);
    let coverage = |k: usize| 1.0 - 2.0 * cdf[k - 1];
    let mut best = None;
    for k in 1..=n.div_ceil(2) {
        if coverage(k) >= level {
            best = Some(k);
        } else {
            break;
        }
    }
    Some(match best {
        Some(k) => MedianInterval {
            estimate,
            lo: Some(s[k - 1]),
            hi: Some(s[n - k]),
            coverage: Some(coverage(k)),
        },
        None => MedianInterval {
            estimate,
            lo: None,
            hi: None,
            coverage: None,
        },
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Group {
    pub harness: String,
    pub model: String,
    pub effort: String,
    /// `live`, `replay` or `none`: never pooled.
    pub mode: String,
    /// Switched-off subsystems; empty for the full harness. Never pooled.
    pub ablation: Vec<String>,
    /// `all` for the pooled group, else the task id.
    pub task: String,
    pub cohorts: Vec<String>,
    pub runs: u32,
    pub successes: u32,
    pub success: Interval,
    pub renders_correct: Option<Interval>,
    pub unavailable: u32,
    /// Medians over runs that reported the value; the counts say how many
    /// did not (an unobserved value is never a zero).
    pub wall_ms: Option<MedianInterval>,
    pub wall_unobserved: u32,
    pub tokens_total: Option<MedianInterval>,
    pub tokens_input: Option<MedianInterval>,
    pub tokens_cached: Option<MedianInterval>,
    pub tokens_output: Option<MedianInterval>,
    pub tokens_unobserved: u32,
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
    /// One verdict per model and effort Aero ran on live.
    pub bar: Vec<ModelVerdict>,
    /// Runs left out of the summary and why (another suite version, a
    /// duplicate run id).
    pub excluded: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Bar {
    /// Every required comparison passes.
    Better,
    /// The evidence is comparable and Aero does not clear the bar.
    NotShown,
    /// The evidence cannot decide (coverage, repeats or telemetry missing).
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Comparison {
    pub against: String,
    pub verdict: Bar,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelVerdict {
    pub model: String,
    pub effort: String,
    pub verdict: Bar,
    pub comparisons: Vec<Comparison>,
    pub reasons: Vec<String>,
}

fn group(task: &str, runs: &[&RunRecord]) -> Group {
    let r0 = runs[0];
    let n = runs.len() as u32;
    let k = runs.iter().filter(|r| r.success).count() as u32;
    let rc: Vec<bool> = runs.iter().filter_map(|r| r.renders_correct).collect();
    let nf: Vec<bool> = runs.iter().filter_map(|r| r.needs_fix).collect();
    let walls: Vec<f64> = runs
        .iter()
        .filter_map(|r| r.wall_ms)
        .map(|w| w as f64)
        .collect();
    let usage: Vec<_> = runs.iter().filter_map(|r| r.tokens).collect();
    let mi = |v: Vec<f64>| median_interval(&v, 0.90);
    let mean = |f: &dyn Fn(&RunRecord) -> f64| runs.iter().map(|r| f(r)).sum::<f64>() / n as f64;
    let gates: Vec<f64> = runs
        .iter()
        .filter_map(|r| r.gate_continuations.map(f64::from))
        .collect();
    let rate = |v: &[bool]| {
        (!v.is_empty())
            .then(|| wilson(v.iter().filter(|b| **b).count() as u32, v.len() as u32, Z90))
    };
    Group {
        harness: r0.harness.clone(),
        model: r0.model.clone(),
        effort: r0.effort.clone(),
        mode: r0.mode.clone(),
        ablation: r0.ablation.clone(),
        task: task.into(),
        cohorts: runs
            .iter()
            .map(|r| r.cohort.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        runs: n,
        successes: k,
        success: wilson(k, n, Z90),
        renders_correct: rate(&rc),
        unavailable: runs.iter().filter(|r| r.unavailable).count() as u32,
        wall_unobserved: n - walls.len() as u32,
        wall_ms: mi(walls),
        tokens_total: mi(usage.iter().map(|u| u.total() as f64).collect()),
        tokens_input: mi(usage.iter().map(|u| u.input as f64).collect()),
        tokens_cached: mi(usage.iter().map(|u| u.cached as f64).collect()),
        tokens_output: mi(usage.iter().map(|u| u.output as f64).collect()),
        tokens_unobserved: n - usage.len() as u32,
        tool_calls_mean: mean(&|r| r.tool_calls as f64),
        retries_mean: mean(&|r| r.retries as f64),
        gate_continuations_mean: (!gates.is_empty())
            .then(|| gates.iter().sum::<f64>() / gates.len() as f64),
        needs_fix: rate(&nf),
    }
}

/// The identity a group never pools across.
type Key = (String, String, String, String, Vec<String>, String);

fn key(r: &RunRecord, task: &str) -> Key {
    (
        r.harness.clone(),
        r.model.clone(),
        r.effort.clone(),
        r.mode.clone(),
        r.ablation.clone(),
        task.to_owned(),
    )
}

/// Summarise runs of one suite version; others (and duplicates) are excluded
/// and listed.
pub fn summarize(runs: &[RunRecord], suite_version: u32) -> Summary {
    let mut excluded = Vec::new();
    let mut keep: Vec<&RunRecord> = Vec::new();
    let mut ids = BTreeSet::new();
    for r in runs {
        if r.suite_version != suite_version {
            excluded.push(format!(
                "{} {} rep {}: suite version {}",
                r.task, r.harness, r.rep, r.suite_version
            ));
        } else if !ids.insert(r.run_id.as_str()) {
            excluded.push(format!(
                "{} {} rep {}: duplicate run id {}",
                r.task, r.harness, r.rep, r.run_id
            ));
        } else {
            keep.push(r);
        }
    }
    let mut by: BTreeMap<Key, Vec<&RunRecord>> = BTreeMap::new();
    for r in &keep {
        by.entry(key(r, "all")).or_default().push(r);
        by.entry(key(r, &r.task)).or_default().push(r);
    }
    let groups: Vec<Group> = by.iter().map(|(k, v)| group(&k.5, v)).collect();
    Summary {
        format: "aero-eval-summary".into(),
        suite_version,
        level: 0.90,
        bar: bar(&keep),
        groups,
        excluded,
    }
}

/// Runs that may enter the bar: live, full harness, a real harness.
fn comparable(r: &RunRecord) -> bool {
    r.mode == "live" && r.ablation.is_empty() && r.harness != "none"
}

/// One verdict per model and effort: Aero against every other harness that
/// ran live on it, each comparison on matched coverage.
pub fn bar(runs: &[&RunRecord]) -> Vec<ModelVerdict> {
    let live: Vec<&RunRecord> = runs.iter().copied().filter(|r| comparable(r)).collect();
    let mut models: BTreeSet<(String, String)> = BTreeSet::new();
    for r in live.iter().filter(|r| r.harness == "aero") {
        models.insert((r.model.clone(), r.effort.clone()));
    }
    let mut out = Vec::new();
    for (model, effort) in models {
        let on: Vec<&RunRecord> = live
            .iter()
            .copied()
            .filter(|r| r.model == model && r.effort == effort)
            .collect();
        let aero: Vec<&RunRecord> = on.iter().copied().filter(|r| r.harness == "aero").collect();
        let others: BTreeSet<&str> = on
            .iter()
            .map(|r| r.harness.as_str())
            .filter(|h| *h != "aero")
            .collect();
        let comparisons: Vec<Comparison> = others
            .iter()
            .map(|h| {
                let o: Vec<&RunRecord> = on.iter().copied().filter(|r| r.harness == *h).collect();
                compare(&aero, &o, h)
            })
            .collect();
        let mut reasons = Vec::new();
        let verdict = if comparisons.is_empty() {
            reasons.push("no other harness ran live on this model and effort".into());
            Bar::Unavailable
        } else if comparisons.iter().any(|c| c.verdict == Bar::NotShown) {
            Bar::NotShown
        } else if comparisons.iter().any(|c| c.verdict == Bar::Unavailable) {
            Bar::Unavailable
        } else {
            Bar::Better
        };
        out.push(ModelVerdict {
            model,
            effort,
            verdict,
            comparisons,
            reasons,
        });
    }
    out
}

/// The verdict of one comparison from its parts: any NotShown wins, then
/// any Unavailable.
fn worst(v: impl IntoIterator<Item = Bar>) -> Bar {
    let v: Vec<Bar> = v.into_iter().collect();
    if v.is_empty() {
        Bar::Unavailable
    } else if v.contains(&Bar::NotShown) {
        Bar::NotShown
    } else if v.contains(&Bar::Unavailable) {
        Bar::Unavailable
    } else {
        Bar::Better
    }
}

/// Aero against one other harness. Runs are compared only inside a cohort
/// both harnesses ran in (one interleaved batch, so drift in load or
/// provider latency hits both alike), and only on a balanced sample: the
/// same tasks, each with the same number of runs on both sides, paired by
/// repeat. Every shared cohort must clear the bar on its own.
fn compare(a: &[&RunRecord], o: &[&RunRecord], against: &str) -> Comparison {
    let cohorts =
        |v: &[&RunRecord]| -> BTreeSet<String> { v.iter().map(|r| r.cohort.clone()).collect() };
    let (ca, co) = (cohorts(a), cohorts(o));
    let shared: Vec<&String> = ca.intersection(&co).collect();
    let mut reasons = Vec::new();
    if shared.is_empty() {
        reasons.push(format!(
            "no shared cohort: aero ran in {ca:?}, {against} in {co:?}; only runs of one interleaved batch compare"
        ));
        return Comparison {
            against: against.into(),
            verdict: Bar::Unavailable,
            reasons,
        };
    }
    let outside = a
        .iter()
        .chain(o)
        .filter(|r| !shared.contains(&&r.cohort))
        .count();
    if outside > 0 {
        reasons.push(format!(
            "{outside} runs outside the shared cohorts are left out"
        ));
    }
    let mut verdicts = Vec::new();
    for c in shared {
        let pa: Vec<&RunRecord> = a.iter().copied().filter(|r| &r.cohort == c).collect();
        let po: Vec<&RunRecord> = o.iter().copied().filter(|r| &r.cohort == c).collect();
        let (bar, why) = compare_cohort(&pa, &po, against);
        verdicts.push(bar);
        reasons.extend(why.into_iter().map(|w| format!("cohort {c}: {w}")));
    }
    Comparison {
        against: against.into(),
        verdict: worst(verdicts),
        reasons,
    }
}

/// The runs of one cohort, sorted by batch position, are one interleaved
/// batch: positions are distinct, start times follow them, repeat blocks
/// never go back, and both harnesses ran in every repeat block.
fn interleaved(a: &[&RunRecord], o: &[&RunRecord]) -> Result<(), String> {
    let mut all: Vec<(&RunRecord, bool)> = a
        .iter()
        .map(|r| (*r, true))
        .chain(o.iter().map(|r| (*r, false)))
        .collect();
    all.sort_by_key(|(r, _)| r.order);
    if all.windows(2).any(|w| w[0].0.order == w[1].0.order) {
        return Err(
            "two runs share a batch position (the cohort was assembled from more than one batch)"
                .into(),
        );
    }
    if all
        .windows(2)
        .any(|w| w[1].0.started_unix_ms < w[0].0.started_unix_ms || w[1].0.rep < w[0].0.rep)
    {
        return Err(
            "its runs do not follow one interleaved batch order (start times or repeats go back)"
                .into(),
        );
    }
    let mut blocks: BTreeMap<u32, (bool, bool)> = BTreeMap::new();
    for (r, is_a) in &all {
        let b = blocks.entry(r.rep).or_default();
        if *is_a {
            b.0 = true;
        } else {
            b.1 = true;
        }
    }
    let lone: Vec<u32> = blocks
        .iter()
        .filter(|(_, (x, y))| !(*x && *y))
        .map(|(k, _)| *k)
        .collect();
    if !lone.is_empty() {
        return Err(format!(
            "repeats {lone:?} ran only one of the two harnesses (not interleaved)"
        ));
    }
    Ok(())
}

fn compare_cohort(a: &[&RunRecord], o: &[&RunRecord], against: &str) -> (Bar, Vec<String>) {
    let mut reasons = Vec::new();
    if let Err(e) = interleaved(a, o) {
        reasons.push(e);
        return (Bar::Unavailable, reasons);
    }
    let by = |v: &[&RunRecord]| -> Result<BTreeMap<String, BTreeMap<u32, usize>>, String> {
        let mut m: BTreeMap<String, BTreeMap<u32, usize>> = BTreeMap::new();
        for (i, r) in v.iter().enumerate() {
            if m.entry(r.task.clone())
                .or_default()
                .insert(r.rep, i)
                .is_some()
            {
                return Err(format!("{} repeat {} ran twice", r.task, r.rep));
            }
        }
        Ok(m)
    };
    let (ta, to) = match (by(a), by(o)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(e), _) | (_, Err(e)) => {
            reasons.push(e);
            return (Bar::Unavailable, reasons);
        }
    };
    let only = |x: &BTreeMap<String, BTreeMap<u32, usize>>,
                y: &BTreeMap<String, BTreeMap<u32, usize>>| {
        x.keys()
            .filter(|k| !y.contains_key(*k))
            .cloned()
            .collect::<Vec<_>>()
    };
    let (oa, oo) = (only(&ta, &to), only(&to, &ta));
    if !oa.is_empty() || !oo.is_empty() {
        reasons.push(format!(
            "coverage differs: only aero ran {oa:?}, only {against} ran {oo:?}"
        ));
        return (Bar::Unavailable, reasons);
    }
    // Repeats both harnesses ran, per task; the balanced sample takes the
    // same number from every task (the first ones, by repeat).
    let paired: BTreeMap<&String, Vec<u32>> = ta
        .iter()
        .map(|(t, reps)| {
            let both: Vec<u32> = reps
                .keys()
                .filter(|r| to[t].contains_key(r))
                .copied()
                .collect();
            (t, both)
        })
        .collect();
    let n = paired.values().map(Vec::len).min().unwrap_or(0);
    if n < MIN_RUNS_PER_TASK {
        let short: Vec<String> = paired
            .iter()
            .filter(|(_, v)| v.len() < MIN_RUNS_PER_TASK)
            .map(|(t, v)| format!("{t} ({})", v.len()))
            .collect();
        reasons.push(format!(
            "fewer than {MIN_RUNS_PER_TASK} repeats ran on both harnesses for {}",
            short.join(", ")
        ));
        return (Bar::Unavailable, reasons);
    }
    let mut sa = Vec::new();
    let mut so = Vec::new();
    for (t, reps) in &paired {
        for r in reps.iter().take(n) {
            sa.push(a[ta[*t][r]]);
            so.push(o[to[*t][r]]);
        }
    }
    let left = a.len() + o.len() - sa.len() - so.len();
    reasons.push(format!(
        "balanced sample: {n} paired repeats per task over {} tasks ({left} runs left out)",
        paired.len()
    ));
    let unobserved = sa
        .iter()
        .chain(&so)
        .filter(|r| r.wall_ms.is_none() || r.tokens.is_none())
        .count();
    if unobserved > 0 {
        reasons.push(format!(
            "{unobserved} runs have no observed wall time or token usage"
        ));
        return (Bar::Unavailable, reasons);
    }
    let rate = |v: &[&RunRecord]| {
        wilson(
            v.iter().filter(|r| r.success).count() as u32,
            v.len() as u32,
            Z90,
        )
    };
    let (ra, ro) = (rate(&sa), rate(&so));
    let higher = ra.lo > ro.hi;
    reasons.push(format!(
        "success {:.0}% [{:.0}, {:.0}] vs {:.0}% [{:.0}, {:.0}]: {}",
        ra.estimate * 100.0,
        ra.lo * 100.0,
        ra.hi * 100.0,
        ro.estimate * 100.0,
        ro.lo * 100.0,
        ro.hi * 100.0,
        if higher {
            "higher, intervals apart"
        } else {
            "not separated"
        }
    ));
    let median = |v: &[&RunRecord], f: &dyn Fn(&RunRecord) -> Option<f64>| {
        median_interval(&v.iter().filter_map(|r| f(r)).collect::<Vec<_>>(), 0.9)
            .map_or(f64::NAN, |m| m.estimate)
    };
    let mut cost_ok = true;
    for (name, f) in [
        (
            "wall ms",
            &(|r: &RunRecord| r.wall_ms.map(|w| w as f64)) as &dyn Fn(&RunRecord) -> Option<f64>,
        ),
        ("tokens", &|r: &RunRecord| {
            r.tokens.map(|u| u.total() as f64)
        }),
    ] {
        let (x, y) = (median(&sa, f), median(&so, f));
        let ok = x <= y;
        cost_ok &= ok;
        reasons.push(format!(
            "median {name} {x:.0} vs {y:.0}: {}",
            if ok { "no worse" } else { "worse" }
        ));
    }
    (
        if higher && cost_ok {
            Bar::Better
        } else {
            Bar::NotShown
        },
        reasons,
    )
}

fn show_median(m: &Option<MedianInterval>, f: impl Fn(f64) -> String) -> String {
    m.map(|i| f(i.estimate)).unwrap_or_else(|| "n/a".into())
}

/// Plain-text table for the terminal.
pub fn render(s: &Summary) -> String {
    let mut out = format!(
        "suite v{} — {:.0}% intervals\n",
        s.suite_version,
        s.level * 100.0
    );
    out.push_str("harness       model           effort  mode    task                          runs  success            median wall   median tokens\n");
    for g in &s.groups {
        let harness = if g.ablation.is_empty() {
            g.harness.clone()
        } else {
            format!("{}-{}", g.harness, g.ablation.join("+"))
        };
        out.push_str(&format!(
            "{:<13} {:<15} {:<7} {:<7} {:<29} {:>4}  {:>3.0}% [{:>3.0},{:>3.0}]  {:>10}  {:>13}\n",
            harness,
            g.model,
            g.effort,
            g.mode,
            g.task,
            g.runs,
            g.success.estimate * 100.0,
            g.success.lo * 100.0,
            g.success.hi * 100.0,
            show_median(&g.wall_ms, |w| format!("{:.1}s", w / 1000.0)),
            show_median(&g.tokens_total, |t| format!("{t:.0}")),
        ));
    }
    for b in &s.bar {
        out.push_str(&format!(
            "aero on {} {}: {}\n",
            b.model,
            b.effort,
            match b.verdict {
                Bar::Better => "BETTER",
                Bar::NotShown => "not shown better",
                Bar::Unavailable => "unavailable",
            }
        ));
        for r in &b.reasons {
            out.push_str(&format!("  {r}\n"));
        }
        for c in &b.comparisons {
            out.push_str(&format!(
                "  vs {}: {:?}\n    {}\n",
                c.against,
                c.verdict,
                c.reasons.join("\n    ")
            ));
        }
    }
    for e in &s.excluded {
        out.push_str(&format!("excluded: {e}\n"));
    }
    out
}
