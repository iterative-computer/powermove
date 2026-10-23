//! Ratchets: every bench's counters against checked-in baselines
//! (`baselines.json` in this crate).
//!
//! A count above its baseline fails. Each bench is held on two scopes: the
//! bench thread's counts (`thread`: the engine's render thread or the
//! export's thread) and every thread's (`all_threads`: decode, encoder and
//! pool threads too), so work moved off the bench thread still counts.
//! Counts are exact; the only slack is a per-bench, per-scope, per-counter
//! relative tolerance listed in the file's `tolerance` table, each with its
//! reason (for counts that depend on another thread's timing).
//!
//! A baseline also records its bench's units (frames, seeks, keys): a bench
//! that measures fewer units fails instead of looking like an improvement.
//! A run of the whole set (`PM_PERF_ONLY` unset) fails when a bench in the
//! file did not run (removed or renamed benches are not silently dropped).
//!
//! `PM_PERF_RATCHET=update` rewrites the file with every count that went down
//! lowered and every new bench recorded; it never raises a baseline, never
//! changes a bench's units, and refuses while a bench in the file did not
//! run (a rename is a hand edit). Raising a baseline is a hand edit of the
//! file, reviewed with the change that needs it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pm_perf::{Counter, Counters};
use serde_json::{Map, Value, json};

use crate::BenchReport;

/// The checked-in baselines.
pub fn baselines_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("baselines.json")
}

/// Which counts a baseline holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// The bench thread's ([`BenchReport::counters`]).
    Thread,
    /// Every thread's ([`BenchReport::global`]).
    AllThreads,
}

impl Scope {
    pub const ALL: [Scope; 2] = [Scope::Thread, Scope::AllThreads];

    pub fn key(self) -> &'static str {
        match self {
            Scope::Thread => "thread",
            Scope::AllThreads => "all_threads",
        }
    }

    fn counts(self, r: &BenchReport) -> &Counters {
        match self {
            Scope::Thread => &r.counters,
            Scope::AllThreads => &r.global,
        }
    }
}

/// One bench's baseline.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BenchBaseline {
    /// Units of work the counts are for, and what a unit is.
    pub units: u32,
    pub unit: String,
    /// Per scope, per counter, the highest count allowed (absent: zero).
    pub counts: BTreeMap<Scope, BTreeMap<String, u64>>,
}

impl BenchBaseline {
    pub fn get(&self, scope: Scope, counter: &str) -> Option<u64> {
        self.counts
            .get(&scope)
            .and_then(|c| c.get(counter).copied())
    }
}

/// A baseline file: per bench, its units and per scope and counter the
/// highest count allowed.
#[derive(Clone, Debug, Default)]
pub struct Baselines {
    pub benches: BTreeMap<String, BenchBaseline>,
    /// (bench, scope, counter) → (relative tolerance, reason).
    pub tolerance: BTreeMap<(String, Scope, String), (f64, String)>,
    /// Counters reported but not gated, with the reason (no correlation
    /// with wall-clock was shown: docs/engine/perfbench.md).
    pub ungated: BTreeMap<String, String>,
}

impl Baselines {
    /// Whether `counter` has an upper bound: a cost counter that is not
    /// listed as ungated (cache hits are benefits, never bounded).
    pub fn gates(&self, counter: Counter) -> bool {
        !counter.is_benefit() && !self.ungated.contains_key(counter.name())
    }

    fn gated(&self) -> Vec<Counter> {
        Counter::ALL
            .iter()
            .copied()
            .filter(|&c| self.gates(c))
            .collect()
    }

    /// The tolerance table's key: the counter's name for the thread scope,
    /// `all_threads/<name>` for every thread's.
    fn tolerance_key(scope: Scope, counter: &str) -> String {
        match scope {
            Scope::Thread => counter.to_owned(),
            Scope::AllThreads => format!("{}/{counter}", scope.key()),
        }
    }

    pub fn read(path: &Path) -> Baselines {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Baselines::default();
        };
        let v: Value = serde_json::from_str(&text).expect("baselines.json parses");
        let mut out = Baselines::default();
        if let Some(benches) = v["benches"].as_object() {
            for (bench, b) in benches {
                let mut entry = BenchBaseline {
                    units: b["units"]
                        .as_u64()
                        .and_then(|u| u32::try_from(u).ok())
                        .unwrap_or_else(|| panic!("{bench}: a baseline records its units")),
                    unit: b["unit"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{bench}: a baseline records its unit"))
                        .to_owned(),
                    counts: BTreeMap::new(),
                };
                for scope in Scope::ALL {
                    let counts = entry.counts.entry(scope).or_default();
                    for (name, n) in b[scope.key()].as_object().into_iter().flatten() {
                        counts.insert(name.clone(), n.as_u64().expect("a count"));
                    }
                }
                out.benches.insert(bench.clone(), entry);
            }
        }
        if let Some(u) = v["ungated"].as_object() {
            for (name, reason) in u {
                out.ungated
                    .insert(name.clone(), reason.as_str().unwrap_or_default().to_owned());
            }
        }
        if let Some(t) = v["tolerance"].as_object() {
            for (bench, counters) in t {
                for (key, spec) in counters.as_object().into_iter().flatten() {
                    let (scope, name) = match key.split_once('/') {
                        Some((s, n)) if s == Scope::AllThreads.key() => (Scope::AllThreads, n),
                        Some(_) => panic!("{bench}: unknown tolerance scope in {key}"),
                        None => (Scope::Thread, key.as_str()),
                    };
                    out.tolerance.insert(
                        (bench.clone(), scope, name.to_owned()),
                        (
                            spec["relative"].as_f64().expect("a relative tolerance"),
                            spec["reason"].as_str().unwrap_or_default().to_owned(),
                        ),
                    );
                }
            }
        }
        out
    }

    pub fn to_json(&self) -> Value {
        let mut benches = Map::new();
        for (bench, b) in &self.benches {
            let mut m = Map::new();
            m.insert("units".into(), json!(b.units));
            m.insert("unit".into(), json!(b.unit));
            for scope in Scope::ALL {
                let counts: Map<String, Value> = b
                    .counts
                    .get(&scope)
                    .into_iter()
                    .flatten()
                    .map(|(k, v)| (k.clone(), json!(v)))
                    .collect();
                m.insert(scope.key().into(), Value::Object(counts));
            }
            benches.insert(bench.clone(), Value::Object(m));
        }
        let mut tolerance = Map::new();
        for ((bench, scope, name), (rel, reason)) in &self.tolerance {
            let e = tolerance
                .entry(bench.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            e[Self::tolerance_key(*scope, name)] = json!({ "relative": rel, "reason": reason });
        }
        json!({
            "about": "Highest allowed pm-perf counts per bench, on the bench thread (thread) and every thread (all_threads), for the recorded units (docs/engine/perfbench.md). Lowered by PM_PERF_RATCHET=update; raised only by hand with the change that needs it.",
            "benches": benches,
            "tolerance": tolerance,
            "ungated": self.ungated,
        })
    }

    pub fn write(&self, path: &Path) {
        let mut text = serde_json::to_string_pretty(&self.to_json()).unwrap();
        text.push('\n');
        std::fs::write(path, text).expect("write baselines.json");
    }
}

/// Why a comparison failed.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// A count above its baseline (`None`: a new bench with no baseline).
    Rise {
        scope: Scope,
        counter: String,
        baseline: Option<u64>,
        actual: u64,
    },
    /// The bench measured other units than its baseline.
    Units { baseline: String, actual: String },
    /// A bench in the file did not run in a run of the whole set.
    DidNotRun,
}

/// One failure.
#[derive(Clone, Debug, PartialEq)]
pub struct Regression {
    pub bench: String,
    pub problem: Problem,
}

impl std::fmt::Display for Regression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bench = &self.bench;
        match &self.problem {
            Problem::Rise {
                scope,
                counter,
                baseline: Some(b),
                actual,
            } => write!(
                f,
                "{bench} {counter} ({}): {actual} > baseline {b} (+{})",
                scope.key(),
                actual - b
            ),
            Problem::Rise {
                scope,
                counter,
                baseline: None,
                actual,
            } => write!(
                f,
                "{bench} {counter} ({}): {actual} has no baseline (PM_PERF_RATCHET=update records a new bench)",
                scope.key()
            ),
            Problem::Units { baseline, actual } => write!(
                f,
                "{bench}: measured {actual}, its baseline is for {baseline} (change the units by hand, with the counts)"
            ),
            Problem::DidNotRun => write!(
                f,
                "{bench}: in baselines.json but did not run (a removed or renamed bench: edit the file by hand)"
            ),
        }
    }
}

/// What a comparison found.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub regressions: Vec<Regression>,
    /// (bench, scope, counter, baseline, actual) where the count went down.
    pub improvements: Vec<(String, Scope, String, u64, u64)>,
}

fn units(r: &BenchReport) -> String {
    format!("{} {}", r.units, r.unit)
}

fn baseline_units(b: &BenchBaseline) -> String {
    format!("{} {}", b.units, b.unit)
}

/// Compares reports with baselines. `whole_set`: every bench was asked to
/// run, so a bench in the file that has no report fails.
pub fn compare(baselines: &Baselines, reports: &[BenchReport], whole_set: bool) -> Outcome {
    let mut out = Outcome::default();
    let gated = baselines.gated();
    for r in reports {
        let base = baselines.benches.get(&r.name);
        if let Some(b) = base
            && (b.units != r.units || b.unit != r.unit)
        {
            out.regressions.push(Regression {
                bench: r.name.clone(),
                problem: Problem::Units {
                    baseline: baseline_units(b),
                    actual: units(r),
                },
            });
            continue;
        }
        for scope in Scope::ALL {
            let counts = scope.counts(r);
            for &c in &gated {
                let actual = counts.get(c);
                let name = c.name();
                let baseline = base.and_then(|b| b.get(scope, name));
                let rise = |baseline| Regression {
                    bench: r.name.clone(),
                    problem: Problem::Rise {
                        scope,
                        counter: name.to_owned(),
                        baseline,
                        actual,
                    },
                };
                match baseline {
                    // A counter the baseline never saw is zero there, unless
                    // the bench itself is new.
                    None if actual == 0 => {}
                    None => out.regressions.push(rise(base.map(|_| 0))),
                    Some(b) => {
                        let slack = baselines
                            .tolerance
                            .get(&(r.name.clone(), scope, name.to_owned()))
                            .map(|(rel, _)| (b as f64 * rel).ceil() as u64)
                            .unwrap_or(0);
                        if actual > b + slack {
                            out.regressions.push(rise(Some(b)));
                        } else if actual < b {
                            out.improvements.push((
                                r.name.clone(),
                                scope,
                                name.to_owned(),
                                b,
                                actual,
                            ));
                        }
                    }
                }
            }
        }
    }
    if whole_set {
        for bench in baselines.benches.keys() {
            if !reports.iter().any(|r| &r.name == bench) {
                out.regressions.push(Regression {
                    bench: bench.clone(),
                    problem: Problem::DidNotRun,
                });
            }
        }
    }
    out
}

/// The baselines lowered to `reports` where they went down, and recorded
/// for new benches. Never raises a count, never changes a bench's units, and
/// never records a counter a known bench did not have. `whole_set`: every
/// bench was asked to run; then a bench of the file without a report is an
/// error (a rename must not record the new name at whatever level it runs).
pub fn lowered(
    baselines: &Baselines,
    reports: &[BenchReport],
    whole_set: bool,
) -> Result<Baselines, String> {
    if whole_set {
        let missing: Vec<&String> = baselines
            .benches
            .keys()
            .filter(|b| !reports.iter().any(|r| &r.name == *b))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "not updating: {missing:?} did not run (a removed or renamed bench: edit baselines.json by hand)"
            ));
        }
    }
    let gated = baselines.gated();
    let mut next = baselines.clone();
    for r in reports {
        let fresh = !next.benches.contains_key(&r.name);
        let entry = next
            .benches
            .entry(r.name.clone())
            .or_insert_with(|| BenchBaseline {
                units: r.units,
                unit: r.unit.to_owned(),
                counts: BTreeMap::new(),
            });
        if entry.units != r.units || entry.unit != r.unit {
            // Units change by hand only.
            continue;
        }
        for scope in Scope::ALL {
            let counts = scope.counts(r);
            let e = entry.counts.entry(scope).or_default();
            // Counters no longer gated are dropped from the file.
            e.retain(|name, _| Counter::from_name(name).is_some_and(|c| gated.contains(&c)));
            for &c in &gated {
                let actual = counts.get(c);
                match e.get(c.name()).copied() {
                    Some(b) if actual < b => {
                        e.insert(c.name().to_owned(), actual);
                    }
                    Some(_) => {}
                    // Zero counts are implied; only a new bench records others.
                    None if fresh && actual > 0 => {
                        e.insert(c.name().to_owned(), actual);
                    }
                    None => {}
                }
            }
            // Counters that went to zero are dropped (zero is implied).
            e.retain(|_, v| *v > 0);
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_perf::Counters;

    fn report(name: &str, draws: u64, allocs: u64) -> BenchReport {
        let mut c = Counters::default();
        c.set(Counter::DrawCalls, draws);
        c.set(Counter::HeapAllocs, allocs);
        BenchReport {
            name: name.into(),
            units: 1,
            unit: "frames",
            counters: c,
            global: c,
            wall_ms: vec![1.0],
            notes: Value::Null,
        }
    }

    fn baselines() -> Baselines {
        let mut b = Baselines::default();
        let counts = BTreeMap::from([
            ("gpu.draw_calls".to_owned(), 10),
            ("cpu.heap_allocs".to_owned(), 100),
        ]);
        b.benches.insert(
            "a".into(),
            BenchBaseline {
                units: 1,
                unit: "frames".into(),
                counts: BTreeMap::from([
                    (Scope::Thread, counts.clone()),
                    (Scope::AllThreads, counts),
                ]),
            },
        );
        for scope in Scope::ALL {
            b.tolerance.insert(
                ("a".into(), scope, "cpu.heap_allocs".into()),
                (0.05, "timing".into()),
            );
        }
        b
    }

    #[test]
    fn rises_fail_drops_lower_and_nothing_raises() {
        let b = baselines();
        // Cache hits are benefits: more of them never fails.
        let mut hits = report("a", 10, 100);
        hits.counters.set(Counter::DrawReuseHits, 500);
        assert!(compare(&b, &[hits], true).regressions.is_empty());
        // Within the tolerance; draws exact.
        let o = compare(&b, &[report("a", 10, 105)], true);
        assert!(o.regressions.is_empty(), "{:?}", o.regressions);
        // Both scopes rise.
        let o = compare(&b, &[report("a", 11, 106)], true);
        assert_eq!(o.regressions.len(), 4);
        // A new bench fails until recorded (and the old one must have run).
        let mut r = report("b", 3, 0);
        assert_eq!(
            compare(&b, std::slice::from_ref(&r), false)
                .regressions
                .len(),
            2
        );
        r.counters.set(Counter::Submits, 2);
        let next = lowered(&b, &[report("a", 8, 120), r.clone()], true).unwrap();
        let a = &next.benches["a"];
        assert_eq!(a.get(Scope::Thread, "gpu.draw_calls"), Some(8));
        assert_eq!(
            a.get(Scope::Thread, "cpu.heap_allocs"),
            Some(100),
            "never raised"
        );
        assert_eq!(next.benches["b"].get(Scope::Thread, "gpu.submits"), Some(2));
        assert_eq!(next.benches["b"].units, 1);
        assert!(
            compare(&next, &[report("a", 8, 100), r], true)
                .regressions
                .is_empty()
        );
        // A counter appearing in a known bench fails, and update does not
        // record it.
        let mut a = report("a", 8, 100);
        a.counters.set(Counter::Readbacks, 1);
        assert_eq!(
            compare(&next, std::slice::from_ref(&a), false)
                .regressions
                .len(),
            1
        );
        assert!(
            lowered(&next, &[a], false).unwrap().benches["a"]
                .get(Scope::Thread, "gpu.readbacks")
                .is_none()
        );
        // Round trip.
        let dir = crate::fixtures::TempDir::new("ratchet-roundtrip");
        let path = dir.path().join("b.json");
        next.write(&path);
        let back = Baselines::read(&path);
        assert_eq!(back.benches, next.benches);
        assert_eq!(back.tolerance, next.tolerance);
    }

    #[test]
    fn work_moved_to_another_thread_still_fails() {
        let b = baselines();
        let mut r = report("a", 10, 100);
        // The bench thread did less; another thread did more.
        r.counters.set(Counter::DrawCalls, 2);
        r.global.set(Counter::DrawCalls, 12);
        let o = compare(&b, &[r], true);
        assert_eq!(o.regressions.len(), 1, "{:?}", o.regressions);
        assert!(matches!(
            &o.regressions[0].problem,
            Problem::Rise {
                scope: Scope::AllThreads,
                ..
            }
        ));
    }

    #[test]
    fn fewer_units_fail_instead_of_improving() {
        let b = baselines();
        let mut r = report("a", 5, 50);
        r.units = 0;
        r.wall_ms.clear();
        let o = compare(&b, &[r.clone()], true);
        assert_eq!(o.regressions.len(), 1);
        assert!(matches!(o.regressions[0].problem, Problem::Units { .. }));
        assert!(o.improvements.is_empty());
        // Update leaves the bench's counts and units alone.
        let next = lowered(&b, &[r], true).unwrap();
        assert_eq!(next.benches["a"], b.benches["a"]);
    }

    #[test]
    fn a_bench_that_did_not_run_fails_and_blocks_update() {
        let b = baselines();
        // Renamed: "a" did not run, "a2" is new.
        let renamed = report("a2", 30, 300);
        let o = compare(&b, std::slice::from_ref(&renamed), true);
        assert!(
            o.regressions
                .iter()
                .any(|r| r.bench == "a" && r.problem == Problem::DidNotRun)
        );
        assert!(lowered(&b, std::slice::from_ref(&renamed), true).is_err());
        // A partial run (PM_PERF_ONLY) checks only what ran.
        assert!(
            compare(&b, &[], false).regressions.is_empty(),
            "nothing ran, nothing asked"
        );
    }
}
