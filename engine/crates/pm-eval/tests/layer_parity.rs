//! Replays `tests/vectors/layer-vectors.json.gz`, written by
//! `tests/vectors/generate-layer-vectors.ts` from the real TypeScript layer
//! queries under bun (JavaScriptCore): 2D and 3D matrices, parenting and
//! group spaces, perspective, homographies, plane hits, depth ordering, pose
//! decomposition, aims, cycles, mattes, size anchors, source times, group
//! spans, transform roots, text animator state and posed glyphs, and the
//! text-animation helpers.
//!
//! Numbers compare within plan section 25's evaluation tolerance
//! (`1e-9 + 1e-9·|b|`; transcendental functions are `libm` here and the
//! platform's in JavaScriptCore); the test also reports how many are bit
//! identical. `PM_LAYER_VECTORS=<file>` replays another set (for example a
//! `PM_LAYER_SCALE=10` run).

use std::collections::BTreeMap;
use std::io::Read;

use pm_model::json::{Value, parse};

#[path = "support/layer_replay.rs"]
mod layer_replay;

use layer_replay::{V, replay};

#[derive(Default)]
struct Stats {
    numbers: usize,
    exact: usize,
    values: usize,
    failures: Vec<String>,
    max_error: f64,
    /// Angles that matched only modulo 360 (see `same`).
    wrapped: usize,
    /// User-facing `Error` messages compared exactly.
    messages: usize,
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
                    && a.iter().all(|(k, x)| {
                        b.get(k).is_some_and(|y| {
                            // Angles decomposed by atan2 at its branch cut: the
                            // platform's and libm's sin and cos can leave the
                            // input's sign on either side of zero, giving 180
                            // and -180 for the same pose.
                            if k.starts_with("rotation")
                                && let (V::N(p), V::N(q)) = (x, y)
                                && ((p - q).abs() - 360.0).abs() <= 1e-9 + 1e-9 * q.abs()
                            {
                                self.numbers += 1;
                                self.wrapped += 1;
                                return true;
                            }
                            self.same(x, y)
                        })
                    })
            }
            // `Error` messages reach the user (toasts, agent tool results):
            // they must match. Engine-internal kinds only need to throw.
            (V::Throw(a), V::Throw(b)) => {
                self.values += 1;
                self.messages += 1;
                a == b
            }
            (V::ThrowInternal(_), V::Throw(_)) => {
                self.values += 1;
                true
            }
            (a, b) => {
                self.values += 1;
                a == b
            }
        }
    }

    fn check(&mut self, what: &dyn Fn() -> String, ours: V, want: &V) {
        if !self.same(&ours, want) && self.failures.len() < 40 {
            self.failures
                .push(format!("{}: ours {ours:?}, want {want:?}", what()));
        }
    }
}

fn load() -> Value {
    let path = std::env::var("PM_LAYER_VECTORS").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/vectors/layer-vectors.json.gz"
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
#[test]
fn layer_queries_match_typescript() {
    let vectors = load();
    let mut stats = Stats::default();
    replay(&vectors, &mut |op, what, ours, want| {
        *stats.ops.entry(op.to_owned()).or_default() += 1;
        stats.check(what, ours, want);
    });

    eprintln!(
        "layer vectors: {} numbers ({} bit-identical, {} angles equal modulo 360, max relative error {:.3e}), {} other values ({} error messages)",
        stats.numbers, stats.exact, stats.wrapped, stats.max_error, stats.values, stats.messages
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
