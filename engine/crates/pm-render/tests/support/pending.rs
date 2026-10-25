//! Explicit, measured ownership of reference gaps. This is a ratchet, not a skip.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub frame: String,
    pub owner: String,
    pub reason: String,
    pub checks: Vec<String>,
    /// All engine, witness and reference samples, plus class facts. Any change
    /// needs review, even when a summary statistic would hide it.
    pub measurement_sha256: String,
}

pub struct Audit {
    entries: BTreeMap<String, Entry>,
    visited: BTreeSet<String>,
    observed: Vec<Entry>,
    errors: Vec<String>,
}

pub fn frame_id(project: &str, time: f64, width: u32, height: u32, side: &str) -> String {
    let mut hash = Sha256::new();
    for text in [project, side] {
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
    }
    hash.update(time.to_bits().to_le_bytes());
    hash.update(width.to_le_bytes());
    hash.update(height.to_le_bytes());
    format!("{:x}", hash.finalize())
}

pub fn measurement(
    engine: &[f64],
    today: &[f64],
    reference: &[f64],
    classes: &[pm_reference::compare::Class],
) -> String {
    let mut hash = Sha256::new();
    for pixels in [engine, today, reference] {
        hash.update((pixels.len() as u64).to_le_bytes());
        for value in pixels {
            // Canonicalize NaN payloads: NaN means unreferenced, not paint.
            let bits = if value.is_nan() {
                f64::NAN.to_bits()
            } else {
                value.to_bits()
            };
            hash.update(bits.to_le_bytes());
        }
    }
    for class in classes {
        hash.update([*class as u8]);
    }
    format!("{:x}", hash.finalize())
}

impl Audit {
    pub fn read(json: &str) -> Self {
        let entries: Vec<Entry> = serde_json::from_str(json).expect("pending fixture");
        let mut map = BTreeMap::new();
        for row in entries {
            assert!(
                matches!(row.owner.as_str(), "L4" | "L6"),
                "pending entry needs an accountable owner"
            );
            assert!(!row.reason.is_empty() && !row.checks.is_empty());
            assert!(
                map.insert(row.frame.clone(), row).is_none(),
                "duplicate pending frame"
            );
        }
        Self {
            entries: map,
            visited: BTreeSet::new(),
            observed: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Only the caller's proven out-of-scope reference gaps are eligible.
    /// Geometry, render errors and preview/export identity never reach here.
    pub fn check(&mut self, row: Entry, eligible: bool) -> bool {
        let known = self.entries.get(&row.frame);
        if known.is_some() {
            self.visited.insert(row.frame.clone());
        }
        if row.checks.is_empty() {
            if known.is_some() {
                self.errors.push(format!(
                    "pending {} resolved: remove its stale entry",
                    row.frame
                ));
            }
            return false;
        }
        self.observed.push(row.clone());
        let Some(expected) = known else {
            return false;
        };
        if !eligible
            || row.owner != expected.owner
            || row.reason != expected.reason
            || row.measurement_sha256 != expected.measurement_sha256
            || row.checks != expected.checks
        {
            self.errors.push(format!(
                "pending {} changed: owner {} must review new measurements",
                row.frame, expected.owner
            ));
            return false;
        }
        eprintln!(
            "PENDING {} owner={} checks={:?}",
            row.frame, expected.owner, row.checks
        );
        true
    }

    pub fn finish(mut self, complete: bool) -> Vec<String> {
        if complete {
            for id in self.entries.keys().filter(|id| !self.visited.contains(*id)) {
                self.errors.push(format!("pending {id} was not measured"));
            }
        }
        if let Ok(path) = std::env::var("PM_CORPUS_PENDING_OBSERVATIONS") {
            let path = std::path::Path::new(&path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, serde_json::to_vec_pretty(&self.observed).unwrap()).unwrap();
        }
        self.errors
    }
}
