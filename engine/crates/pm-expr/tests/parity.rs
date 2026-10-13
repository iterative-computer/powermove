//! Bit-for-bit parity with `legacy/core/expression.ts` running in V8.
//!
//! `tests/fixtures/parity-*.jsonl` hold public cases (edge cases, a coercion
//! matrix, expressions found in the repository, and seeded fuzz cases) with
//! the results V8 15.2 (Electron 44) produced; `parity/run.ts` regenerates them
//! and, with `PM_EXPR_PARITY_DIR` set, adds a larger fuzz run, the private
//! corpus, and a sweep of the `Math` functions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod support;

use support::*;

use pm_expr::{Dialect, Expression, JsStr, JsonHost, Root, Step, Value};
use pm_model::json::{Value as Json, parse};

struct Suite {
    contexts: Vec<Json>,
    cases: Vec<(Vec<u16>, usize, String)>,
    expected: Vec<Json>,
}

fn load(cases: &Path, expected: &Path) -> Suite {
    let text =
        std::fs::read_to_string(cases).unwrap_or_else(|e| panic!("{}: {e}", cases.display()));
    let mut lines = text.lines();
    let header = parse(lines.next().expect("header")).expect("header json");
    let contexts = match header.as_object().and_then(|h| h.get("contexts")) {
        Some(Json::Array(items)) => items.clone(),
        _ => panic!("no contexts"),
    };
    let cases = lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            let v = parse(l).expect("case json");
            let o = v.as_object().expect("case object");
            let source: Vec<u16> = o
                .get("source")
                .and_then(Json::as_js_string)
                .expect("source")
                .utf16_units()
                .collect();
            let ctx = o.get("ctx").and_then(Json::as_f64).expect("ctx") as usize;
            let origin = o
                .get("origin")
                .and_then(Json::as_str)
                .unwrap_or("")
                .to_string();
            (source, ctx, origin)
        })
        .collect::<Vec<_>>();
    let expected = std::fs::read_to_string(expected)
        .unwrap_or_else(|e| panic!("{}: {e}", expected.display()))
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| parse(l).expect("expected json"))
        .collect::<Vec<_>>();
    assert_eq!(
        cases.len(),
        expected.len(),
        "cases and results differ in length"
    );
    Suite {
        contexts,
        cases,
        expected,
    }
}

fn check_suite(suite: &Suite, label: &str) {
    let mut failures = Vec::new();
    let mut per_origin: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for ((source, ctx, origin), expected) in suite.cases.iter().zip(&suite.expected) {
        let spec = &suite.contexts[*ctx];
        let got = evaluate_case(source, spec, Dialect::Legacy);
        let kind = origin.split(':').next().unwrap_or("").to_string();
        let entry = per_origin.entry(kind).or_default();
        entry.0 += 1;
        if &got != expected {
            entry.1 += 1;
            let source = String::from_utf16_lossy(source);
            failures.push(format!(
                "[{origin}] ctx {ctx} {source:?}\n  expected {expected:?}\n  got      {got:?}"
            ));
        }
        if !mentions_frame(source) {
            let native = evaluate_case(source, spec, Dialect::Native);
            if native != got {
                let source = String::from_utf16_lossy(source);
                failures.push(format!(
                    "[native differs] {source:?}\n  legacy {got:?}\n  native {native:?}"
                ));
            }
        }
    }
    for (origin, (total, bad)) in &per_origin {
        println!("{label}: {origin}: {} of {total} cases match", total - bad);
    }
    if !failures.is_empty() {
        for f in failures.iter().take(40) {
            println!("{f}");
        }
        panic!("{label}: {} mismatches", failures.len());
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn fixture_matches_v8() {
    let dir = fixtures();
    let suite = load(
        &dir.join("parity-cases.jsonl"),
        &dir.join("parity-expected.jsonl"),
    );
    assert!(suite.cases.len() > 10_000, "fixture is unexpectedly small");
    check_suite(&suite, "fixture");
}

#[test]
fn external_run_matches_v8() {
    let Ok(dir) = std::env::var("PM_EXPR_PARITY_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let suite = load(&dir.join("cases.jsonl"), &dir.join("expected.jsonl"));
    check_suite(&suite, "run");
}

// ── Math sweep ──────────────────────────────────────────────────────────────

#[test]
fn external_math_sweep_matches_v8() {
    let Ok(dir) = std::env::var("PM_EXPR_PARITY_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let Ok(meta) = std::fs::read_to_string(dir.join("math.json")) else {
        return;
    };
    let meta = parse(&meta).unwrap();
    let count = meta.field("count").and_then(Json::as_f64).unwrap() as usize;
    let seed = meta.field("seed").and_then(Json::as_f64).unwrap() as u32;
    let Some(Json::Array(functions)) = meta.field("functions") else {
        panic!("functions")
    };
    let mut total_bad = 0;
    for (index, fun) in functions.iter().enumerate() {
        let fun = fun.as_str().unwrap();
        let bytes = std::fs::read(dir.join(format!("math-{fun}.bin"))).unwrap();
        assert_eq!(bytes.len(), count * 8);
        let mut rng = Mulberry32(seed ^ index as u32);
        let mut bad = Vec::new();
        for i in 0..count {
            let (a, b) = sample(fun, i, &mut rng);
            let expected = f64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap());
            let got = math_apply(fun, a, b);
            let same = got.to_bits() == expected.to_bits() || (got.is_nan() && expected.is_nan());
            if !same {
                bad.push((a, b, expected, got, i % 5));
            }
        }
        let by_mode: Vec<usize> = (0..5)
            .map(|m| bad.iter().filter(|x| x.4 == m).count())
            .collect();
        println!(
            "math: {fun}: {} of {count} samples identical (mismatches by input mode {by_mode:?})",
            count - bad.len()
        );
        let examples = (0..5).filter_map(|m| bad.iter().find(|x| x.4 == m));
        for (a, b, e, g, _) in examples {
            println!(
                "  {fun}({a:e}, {b:e}): V8 {e:e} ({:016x}), Rust {g:e} ({:016x})",
                e.to_bits(),
                g.to_bits()
            );
        }
        total_bad += bad.len();
    }
    assert_eq!(total_bad, 0, "math sweep mismatches");
}

// ── dependency soundness ────────────────────────────────────────────────────

/// Every node path of a JSON tree, with the steps evaluation would record.
fn paths(value: &Json, prefix: &mut Vec<Step>, out: &mut Vec<Vec<Step>>) {
    out.push(prefix.clone());
    match value {
        Json::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                prefix.push(Step::Index(i as u64));
                paths(item, prefix, out);
                prefix.pop();
            }
        }
        Json::Object(map) => {
            for (k, v) in map.iter() {
                prefix.push(Step::Key(JsStr::from_units(
                    k.utf16_units().collect::<Vec<u16>>(),
                )));
                paths(v, prefix, out);
                prefix.pop();
            }
        }
        _ => {}
    }
}

fn node_mut<'a>(mut value: &'a mut Json, path: &[Step]) -> &'a mut Json {
    for step in path {
        value = match (value, step) {
            (Json::Array(items), Step::Index(i)) => &mut items[*i as usize],
            (Json::Object(map), Step::Key(k)) => map
                .get_key_mut(&pm_model::JsString::from_utf16_units(k.units()))
                .unwrap(),
            _ => unreachable!(),
        };
    }
    value
}

fn perturb(value: &mut Json) {
    *value = match value {
        Json::Number(n) => Json::Number(*n + 1.5),
        Json::String(_) => Json::String("perturbed".into()),
        Json::Bool(b) => Json::Bool(!*b),
        Json::Null => Json::Number(3.0),
        Json::Array(_) | Json::Object(_) => Json::String("[replaced]".into()),
    };
}

fn outcome(expr: &Expression, spec: &Json) -> (Json, pm_expr::Dependencies) {
    let mut ctx = context(spec, 7.0);
    if !expr.needs_idx() {
        ctx.idx = 0.0;
    }
    let out = expr.evaluate(&JsonHost::new(), &ctx);
    // An object result is the object itself (the channel keeps its keyframed
    // value); reading its contents to print it happens outside the evaluation.
    let encoded = match &out.result {
        Ok(Value::Object(_)) => js("object"),
        Ok(v) => encode(v),
        Err(e) => js(&e.message()),
    };
    (encoded, out.deps)
}

/// Changing anything an evaluation did not record must not change its result.
#[test]
fn unrecorded_inputs_do_not_change_results() {
    let dir = fixtures();
    let suite = load(
        &dir.join("parity-cases.jsonl"),
        &dir.join("parity-expected.jsonl"),
    );
    let mut rng = Mulberry32(99);
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for (source, ctx, _) in suite.cases.iter().step_by(3) {
        let Some(expr) = Expression::compile_utf16(source, Dialect::Native) else {
            continue;
        };
        let source = String::from_utf16_lossy(source);
        let spec = &suite.contexts[*ctx];
        let (base, deps) = outcome(&expr, spec);

        // Context scalars.
        use pm_expr::ContextReads as C;
        let scalars: [(&str, C); 5] = [
            ("t", C::LOCAL_TIME),
            ("T", C::TIME),
            ("fps", C::FPS),
            ("ch", C::CHANNEL),
            ("idx", C::INDEX),
        ];
        for (key, flag) in scalars {
            if deps.context.contains(flag) {
                continue;
            }
            let mut changed = spec.clone();
            if let Json::Object(m) = &mut changed {
                let v = if key == "ch" {
                    Json::String("other.channel".into())
                } else {
                    Json::Number(41.25)
                };
                m.insert(key, v);
            }
            checked += 1;
            if outcome(&expr, &changed).0 != base {
                failures.push(format!("{source:?}: changing {key} changed the result"));
            }
        }

        // Document fields under comp (and layer, which may alias comp.layers[i]).
        let layer_index = spec
            .field("layerIndex")
            .and_then(Json::as_f64)
            .map(|i| i as u64);
        for root in ["comp", "layer", "value"] {
            let Some(tree) = spec.field(root) else {
                continue;
            };
            let mut all = Vec::new();
            paths(tree, &mut Vec::new(), &mut all);
            let root_kind = match root {
                "comp" => Root::Comp,
                "layer" => Root::Layer,
                _ => Root::Value,
            };
            let candidates: Vec<&Vec<Step>> = all
                .iter()
                .filter(|p| !p.is_empty())
                .filter(|p| !deps.affected_by(root_kind, p))
                .filter(|p| {
                    // comp.layers[i].… is layer.… when the layer is that element.
                    match (root_kind, layer_index, p.as_slice()) {
                        // The context finds its layer through comp.layers.
                        (Root::Comp, Some(_), [Step::Key(k)]) if k.eq_str("layers") => false,
                        (Root::Comp, Some(i), [Step::Key(k), Step::Index(j), rest @ ..])
                            if k.eq_str("layers") && *j == i =>
                        {
                            !deps.affected_by(Root::Layer, rest)
                        }
                        _ => true,
                    }
                })
                .collect();
            for _ in 0..4.min(candidates.len()) {
                let path = candidates[rng.next() as usize % candidates.len()];
                let mut changed = spec.clone();
                if let Json::Object(m) = &mut changed {
                    perturb(node_mut(m.get_mut(root).unwrap(), path));
                }
                checked += 1;
                if outcome(&expr, &changed).0 != base {
                    failures.push(format!(
                        "{source:?}: changing {root}{path:?} changed the result"
                    ));
                }
            }
        }
    }
    println!("dependency soundness: {checked} perturbations checked");
    for f in failures.iter().take(30) {
        println!("{f}");
    }
    assert!(
        failures.is_empty(),
        "{} perturbations changed results",
        failures.len()
    );
}
