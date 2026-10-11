//! Spike S8: cross-process GPU contention and GPU time slicing.
//!
//! Plan: `docs/rust-engine.md` section 8 (GPU time slicing, contract 13), section 19
//! (Memory and GPU), Phase 0 row S8, section 25 S8 row. Report: `docs/engine/spikes/S8.md`.
//!
//! Three binaries share this library:
//! - `s8-interactive` (process A): paces 4K frames at a display rate and measures drops.
//! - `s8-job` (process B): an export-like workload, unsliced or time sliced, plus
//!   optional VideoToolbox encode sessions.
//! - `s8-run`: the orchestrator. Spawns A and B, relays nothing itself (A writes the
//!   deadline signals straight into a FIFO that B reads, standing in for main's relay),
//!   collects the JSON summaries.

pub mod blender;
pub mod clock;
pub mod display;
pub mod gpu;
pub mod present;
pub mod signal;
pub mod stats;
pub mod sys;
pub mod vt;

/// Parse `--key value` and `--flag` style arguments into a small lookup table.
pub struct Args {
    items: Vec<(String, Option<String>)>,
}

impl Args {
    pub fn from_env() -> Self {
        Self::parse(std::env::args().skip(1))
    }

    pub fn parse(iter: impl IntoIterator<Item = String>) -> Self {
        let raw: Vec<String> = iter.into_iter().collect();
        let mut items = Vec::new();
        let mut i = 0;
        while i < raw.len() {
            let key = raw[i].trim_start_matches("--").to_string();
            if i + 1 < raw.len() && !raw[i + 1].starts_with("--") {
                items.push((key, Some(raw[i + 1].clone())));
                i += 2;
            } else {
                items.push((key, None));
                i += 1;
            }
        }
        Self { items }
    }

    pub fn flag(&self, key: &str) -> bool {
        self.items.iter().any(|(k, _)| k == key)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        self.items
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_deref())
    }

    pub fn string_or(&self, key: &str, default: &str) -> String {
        self.str(key).unwrap_or(default).to_string()
    }

    pub fn num<T: std::str::FromStr>(&self, key: &str, default: T) -> T {
        match self.str(key) {
            Some(v) => v.parse().unwrap_or_else(|_| panic!("--{key}: cannot parse {v:?}")),
            None => default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_parse_values_and_flags() {
        let a = Args::parse(
            ["--seconds", "60", "--rt", "--mode", "sliced", "--slice-ms", "2.5"]
                .iter()
                .map(|s| s.to_string()),
        );
        assert_eq!(a.num::<u32>("seconds", 0), 60);
        assert!(a.flag("rt"));
        assert!(!a.flag("missing"));
        assert_eq!(a.str("mode"), Some("sliced"));
        assert_eq!(a.num::<f64>("slice-ms", 0.0), 2.5);
        assert_eq!(a.num::<u32>("absent", 7), 7);
    }
}
