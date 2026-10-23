//! Ratchets: every bench's counters against checked-in baselines
//! (`baselines.json` in this crate).
//!
//! A count above its baseline fails. Counts are exact; the only slack is a
//! per-bench, per-counter relative tolerance listed in the file's
//! `tolerance` table, each with its reason (for counts that depend on
//! another thread's timing). `PM_PERF_RATCHET=update` rewrites the file
//! with every count that went down lowered and every new bench or counter
//! recorded; it never raises a baseline. Raising one is a hand edit of the
//! file, reviewed with the change that needs it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pm_perf::Counter;
use serde_json::{Map, Value, json};

use crate::BenchReport;

/// The checked-in baselines.
pub fn baselines_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("baselines.json")
}

/// A baseline file: per bench, per counter, the highest count allowed.
#[derive(Clone, Debug, Default)]
pub struct Baselines {
    pub benches: BTreeMap<String, BTreeMap<String, u64>>,
    /// (bench, counter) → (relative tolerance, reason).
    pub tolerance: BTreeMap<(String, String), (f64, String)>,
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
}

impl Baselines {
    pub fn read(path: &Path) -> Baselines {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Baselines::default();
        };
        let v: Value = serde_json::from_str(&text).expect("baselines.json parses");
        let mut out = Baselines::default();
        if let Some(benches) = v["benches"].as_object() {
            for (bench, counters) in benches {
                let entry = out.benches.entry(bench.clone()).or_default();
                for (name, n) in counters.as_object().into_iter().flatten() {
                    entry.insert(name.clone(), n.as_u64().expect("a count"));
                }
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
                for (name, spec) in counters.as_object().into_iter().flatten() {
                    out.tolerance.insert(
                        (bench.clone(), name.clone()),
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
        for (bench, counters) in &self.benches {
            let m: Map<String, Value> = counters
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            benches.insert(bench.clone(), Value::Object(m));
        }
        let mut tolerance = Map::new();
        for ((bench, name), (rel, reason)) in &self.tolerance {
            let e = tolerance
                .entry(bench.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            e[name] = json!({ "relative": rel, "reason": reason });
        }
        json!({
            "about": "Highest allowed pm-perf counts per bench (docs/engine/perfbench.md). Lowered by PM_PERF_RATCHET=update; raised only by hand with the change that needs it.",
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

/// One counter above its baseline.
#[derive(Clone, Debug, PartialEq)]
pub struct Regression {
    pub bench: String,
    pub counter: String,
    pub baseline: Option<u64>,
    pub actual: u64,
}

impl std::fmt::Display for Regression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.baseline {
            Some(b) => write!(
                f,
                "{} {}: {} > baseline {} (+{})",
                self.bench,
                self.counter,
                self.actual,
                b,
                self.actual - b
            ),
            None => write!(
                f,
                "{} {}: {} has no baseline (PM_PERF_RATCHET=update records it)",
                self.bench, self.counter, self.actual
            ),
        }
    }
}

/// What a comparison found.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub regressions: Vec<Regression>,
    /// (bench, counter, baseline, actual) where the count went down.
    pub improvements: Vec<(String, String, u64, u64)>,
}

/// Compares reports with baselines.
pub fn compare(baselines: &Baselines, reports: &[BenchReport]) -> Outcome {
    let mut out = Outcome::default();
    for r in reports {
        let base = baselines.benches.get(&r.name);
        for &c in Counter::ALL.iter().filter(|&&c| baselines.gates(c)) {
            let actual = r.counters.get(c);
            let name = c.name();
            let baseline = base.and_then(|b| b.get(name).copied());
            match baseline {
                // A counter the baseline never saw is zero there, unless the
                // bench itself is new.
                None if actual == 0 => {}
                None => out.regressions.push(Regression {
                    bench: r.name.clone(),
                    counter: name.to_owned(),
                    baseline: base.map(|_| 0),
                    actual,
                }),
                Some(b) => {
                    let slack = baselines
                        .tolerance
                        .get(&(r.name.clone(), name.to_owned()))
                        .map(|(rel, _)| (b as f64 * rel).ceil() as u64)
                        .unwrap_or(0);
                    if actual > b + slack {
                        out.regressions.push(Regression {
                            bench: r.name.clone(),
                            counter: name.to_owned(),
                            baseline: Some(b),
                            actual,
                        });
                    } else if actual < b {
                        out.improvements
                            .push((r.name.clone(), name.to_owned(), b, actual));
                    }
                }
            }
        }
    }
    out
}

/// The baselines lowered to `reports` where they went down, and recorded
/// for benches and counters they did not have. Never raises a count.
pub fn lowered(baselines: &Baselines, reports: &[BenchReport]) -> Baselines {
    let mut next = baselines.clone();
    for r in reports {
        let entry = next.benches.entry(r.name.clone()).or_default();
        let fresh = entry.is_empty();
        let gated: Vec<Counter> = Counter::ALL
            .iter()
            .copied()
            .filter(|&c| baselines.gates(c))
            .collect();
        // Counters no longer gated are dropped from the file.
        entry.retain(|name, _| Counter::from_name(name).is_some_and(|c| gated.contains(&c)));
        for c in gated {
            let actual = r.counters.get(c);
            match entry.get(c.name()).copied() {
                Some(b) if actual < b => {
                    entry.insert(c.name().to_owned(), actual);
                }
                Some(_) => {}
                // Zero counts are implied; only a new bench records others.
                None if fresh && actual > 0 => {
                    entry.insert(c.name().to_owned(), actual);
                }
                None => {}
            }
        }
        // Counters that went to zero are dropped (zero is implied).
        entry.retain(|_, v| *v > 0);
    }
    next
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

    #[test]
    fn rises_fail_drops_lower_and_nothing_raises() {
        let mut b = Baselines::default();
        b.benches.insert(
            "a".into(),
            BTreeMap::from([
                ("gpu.draw_calls".into(), 10),
                ("cpu.heap_allocs".into(), 100),
            ]),
        );
        b.tolerance.insert(
            ("a".into(), "cpu.heap_allocs".into()),
            (0.05, "timing".into()),
        );
        // Cache hits are benefits: more of them never fails.
        let mut hits = report("a", 10, 100);
        hits.counters.set(Counter::DrawReuseHits, 500);
        assert!(compare(&b, &[hits]).regressions.is_empty());
        // Within the tolerance; draws exact.
        let o = compare(&b, &[report("a", 10, 105)]);
        assert!(o.regressions.is_empty(), "{:?}", o.regressions);
        let o = compare(&b, &[report("a", 11, 106)]);
        assert_eq!(o.regressions.len(), 2);
        // A new bench and a new counter fail until recorded.
        let mut r = report("b", 3, 0);
        assert_eq!(compare(&b, std::slice::from_ref(&r)).regressions.len(), 1);
        r.counters.set(Counter::Submits, 2);
        let next = lowered(&b, &[report("a", 8, 120), r.clone()]);
        assert_eq!(next.benches["a"]["gpu.draw_calls"], 8);
        assert_eq!(next.benches["a"]["cpu.heap_allocs"], 100, "never raised");
        assert_eq!(next.benches["b"]["gpu.submits"], 2);
        assert!(compare(&next, &[r]).regressions.is_empty());
        // A counter appearing in a known bench fails, and update does not
        // record it.
        let mut a = report("a", 8, 100);
        a.counters.set(Counter::Readbacks, 1);
        assert_eq!(
            compare(&next, std::slice::from_ref(&a)).regressions.len(),
            1
        );
        assert!(!lowered(&next, &[a]).benches["a"].contains_key("gpu.readbacks"));
        // Round trip.
        let dir = crate::fixtures::TempDir::new("ratchet-roundtrip");
        let path = dir.path().join("b.json");
        next.write(&path);
        let back = Baselines::read(&path);
        assert_eq!(back.benches, next.benches);
        assert_eq!(back.tolerance.len(), 1);
    }
}
