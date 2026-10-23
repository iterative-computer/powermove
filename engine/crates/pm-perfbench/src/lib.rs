//! Deterministic engine benches and their ratchets (docs/engine/perfbench.md).
//!
//! Each bench drives the engine through one user-facing workload on public,
//! synthetic fixtures ([`fixtures`]) and reports the work it did as
//! [`pm_perf`] counters (render passes, draw calls, pipeline and bind group
//! creations, GPU allocations, uploads, readbacks, shader compiles, heap
//! allocations, cache hits and misses), plus wall-clock time per unit
//! (informational: this machine is shared and noisy). The counts are
//! deterministic, so [`ratchet`] can hold every bench to a checked-in
//! baseline and lower it when the engine does less.

pub mod fixtures;
pub mod ratchet;

#[cfg(target_os = "macos")]
pub mod benches;

use pm_perf::Counters;

/// What one bench measured.
#[derive(Clone, Debug)]
pub struct BenchReport {
    /// The bench's name (`ratchet` baseline key).
    pub name: String,
    /// Units of work measured (frames, seeks, keys).
    pub units: u32,
    /// What a unit is.
    pub unit: &'static str,
    /// The bench thread's counters over every measured unit (the ratchet's
    /// numbers: the engine's render thread does this work).
    pub counters: Counters,
    /// Every thread's counters over the same span (decode, encode and pool
    /// threads included; informational).
    pub global: Counters,
    /// Wall-clock time of each unit, GPU work included (ms, informational).
    pub wall_ms: Vec<f64>,
    /// Bench-specific facts (fixture summary, sizes).
    pub notes: serde_json::Value,
}

impl BenchReport {
    /// The wall-clock percentile `p` (0..=1) of the units (ms).
    pub fn wall_at(&self, p: f64) -> f64 {
        let mut xs = self.wall_ms.clone();
        if xs.is_empty() {
            return 0.0;
        }
        xs.sort_by(f64::total_cmp);
        xs[((xs.len() - 1) as f64 * p).round() as usize]
    }

    pub fn wall_total(&self) -> f64 {
        self.wall_ms.iter().sum()
    }

    pub fn to_json(&self) -> serde_json::Value {
        let counters: serde_json::Map<String, serde_json::Value> = self
            .counters
            .nonzero()
            .map(|(c, v)| (c.name().to_owned(), v.into()))
            .collect();
        let global: serde_json::Map<String, serde_json::Value> = self
            .global
            .nonzero()
            .map(|(c, v)| (c.name().to_owned(), v.into()))
            .collect();
        serde_json::json!({
            "name": self.name,
            "units": self.units,
            "unit": self.unit,
            "counters": counters,
            "global": global,
            "wall_ms": {
                "p50": self.wall_at(0.5), "p95": self.wall_at(0.95), "max": self.wall_at(1.0),
                "total": self.wall_total()
            },
            "notes": self.notes,
        })
    }
}
