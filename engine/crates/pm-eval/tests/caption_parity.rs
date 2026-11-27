//! Replays `tests/vectors/caption-vectors.json.gz`, written by
//! `tests/vectors/generate-caption-vectors.ts` from the captions model,
//! layout, and rasterizer frame under bun: style normalization, tokens, word
//! timings, layout, placement, word states, cue lookup, and whole frames
//! (stored, normalized, and missing styles; idle bounds; throws).
//!
//! Text is measured by the same synthetic canvas on both sides ([`Synth`]),
//! which depends on every part of the font string and the letter spacing.
//! Numbers compare within plan section 25's evaluation tolerance; the test
//! reports how many are bit identical. `PM_CAPTION_VECTORS=<file>` replays
//! another set.

use std::collections::BTreeMap;
use std::io::Read;

use pm_model::json::{Value, parse};

#[path = "support/caption_replay.rs"]
mod caption_replay;
#[path = "support/layer_replay.rs"]
mod layer_replay;

use layer_replay::V;

fn load() -> Value {
    let path = std::env::var("PM_CAPTION_VECTORS").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/vectors/caption-vectors.json.gz"
        )
        .to_owned()
    });
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut text = String::new();
    flate2::read::GzDecoder::new(&bytes[..])
        .read_to_string(&mut text)
        .unwrap();
    parse(&text).unwrap()
}

#[derive(Default)]
struct Stats {
    numbers: usize,
    exact: usize,
    max_error: f64,
    values: usize,
    failures: Vec<String>,
    ops: BTreeMap<String, usize>,
}

impl Stats {
    fn same(&mut self, ours: &V, want: &V) -> bool {
        match (ours, want) {
            (V::N(a), V::N(b)) => {
                self.numbers += 1;
                if a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()) {
                    self.exact += 1;
                    return true;
                }
                if !a.is_finite() || !b.is_finite() {
                    return false;
                }
                let e = (a - b).abs();
                let ok = e <= 1e-9 + 1e-9 * b.abs();
                if ok {
                    self.max_error = self.max_error.max(e / (1.0 + b.abs()));
                }
                ok
            }
            (V::A(a), V::A(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| self.same(x, y))
            }
            (V::O(a), V::O(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, x)| b.get(k).is_some_and(|y| self.same(x, y)))
            }
            // The legacy throws are engine TypeErrors (JavaScriptCore's
            // messages); the port throws a TypeError with its own text.
            (V::Throw(a), V::Throw(_)) => {
                self.values += 1;
                a.starts_with("TypeError")
            }
            (a, b) => {
                self.values += 1;
                a == b
            }
        }
    }
}
#[test]
fn captions_match_typescript() {
    let vectors = load();
    let mut stats = Stats::default();
    caption_replay::replay(&vectors, &mut |op, case, ours, want| {
        *stats.ops.entry(op.to_owned()).or_default() += 1;
        if !stats.same(&ours, want) && stats.failures.len() < 30 {
            stats.failures.push(format!(
                "{op} {}: ours {ours:?}, want {want:?}",
                pm_model::json::to_canonical_string(case)
                    .chars()
                    .take(600)
                    .collect::<String>()
            ));
        }
    });
    eprintln!(
        "caption vectors: {} numbers ({} bit-identical, max relative error {:.3e}), {} other values",
        stats.numbers, stats.exact, stats.max_error, stats.values
    );
    eprintln!("ops: {:?}", stats.ops);
    for f in &stats.failures {
        eprintln!("  {f}");
    }
    assert!(
        stats.failures.is_empty(),
        "{} mismatches",
        stats.failures.len()
    );
}
