//! Bit-for-bit parity with `legacy/core/expression.ts` running in V8.
//!
//! `tests/fixtures/parity-*.jsonl` hold public cases (edge cases, a coercion
//! matrix, expressions found in the repository, and seeded fuzz cases) with
//! the results V8 15.2 (Electron 44) produced; `parity/run.ts` regenerates them
//! and, with `PM_EXPR_PARITY_DIR` set, adds a larger fuzz run, the private
//! corpus, and a sweep of the `Math` functions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pm_expr::{
    Context, Dialect, Expression, JsStr, JsonHost, JsonRef, Root, Step, Value, evaluate_channel,
    expression_diagnostic_utf16, jsmath, to_js_string,
};
use pm_model::json::{Map, Value as Json, parse};

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

trait JsonExt {
    fn field(&self, key: &str) -> Option<&Json>;
}

impl JsonExt for Json {
    fn field(&self, key: &str) -> Option<&Json> {
        self.as_object().and_then(|m| m.get(key))
    }
}

fn context(spec: &Json, frame: f64) -> Context<JsonRef<'_>> {
    let num = |k: &str| spec.field(k).and_then(Json::as_f64).unwrap_or(f64::NAN);
    let comp = spec.field("comp").expect("comp");
    let layer = match spec.field("layerIndex").and_then(Json::as_f64) {
        Some(i) => match comp.field("layers") {
            Some(Json::Array(items)) => JsonHost::value(&items[i as usize]),
            _ => panic!("layerIndex without layers"),
        },
        None => spec
            .field("layer")
            .map_or(Value::Undefined, JsonHost::value),
    };
    Context {
        t: num("t"),
        time: num("T"),
        fps: num("fps"),
        frame,
        value: spec
            .field("value")
            .map_or(Value::Undefined, JsonHost::value),
        layer,
        comp: JsonHost::value(comp),
        channel: JsStr::from(spec.field("ch").and_then(Json::as_str).unwrap_or("")),
        idx: num("idx"),
    }
}

fn js(s: &str) -> Json {
    Json::String(s.into())
}

fn jstr(s: &JsStr) -> Json {
    Json::String(pm_model::JsString::from_utf16_units(s.units()))
}

fn obj(entries: Vec<(&str, Json)>) -> Json {
    let mut m = Map::new();
    for (k, v) in entries {
        m.insert(k, v);
    }
    Json::Object(m)
}

fn encode<'a>(value: &Value<JsonRef<'a>>) -> Json {
    match value {
        Value::Undefined => obj(vec![("type", js("undefined"))]),
        Value::Null => obj(vec![("type", js("null"))]),
        Value::Bool(b) => obj(vec![("type", js("boolean")), ("value", Json::Bool(*b))]),
        Value::Number(n) => {
            let bits = if n.is_nan() {
                "nan".to_string()
            } else {
                format!("{:016x}", n.to_bits())
            };
            obj(vec![
                ("type", js("number")),
                ("bits", Json::String(bits.into())),
            ])
        }
        Value::String(s) => obj(vec![("type", js("string")), ("value", jstr(s))]),
        Value::Object(_) => {
            let (string, error) = match to_js_string(&JsonHost::new(), value) {
                Ok(s) => (jstr(&s), Json::Null),
                Err(e) => (Json::Null, js(&e.message())),
            };
            obj(vec![
                ("type", js(value.type_of())),
                ("string", string),
                ("error", error),
            ])
        }
    }
}

/// The record `reference.mjs` writes for one case.
fn evaluate_case(source: &[u16], spec: &Json, dialect: Dialect) -> Json {
    let mut fields = vec![(
        "diagnostic",
        expression_diagnostic_utf16(source, dialect).map_or(Json::Null, |d| js(&d)),
    )];
    let compiled = Expression::compile_utf16(source, dialect);
    fields.push(("compiled", Json::Bool(compiled.is_some())));
    let mut ctx = context(spec, 0.0);
    if let Some(expr) = &compiled {
        fields.push(("needsIdx", Json::Bool(expr.needs_idx())));
        if !expr.needs_idx() {
            ctx.idx = 0.0;
        }
        let out = expr.evaluate(&JsonHost::new(), &ctx);
        fields.push((
            "result",
            match &out.result {
                Ok(v) => encode(v),
                Err(e) => obj(vec![("type", js("throw")), ("message", js(&e.message()))]),
            },
        ));
    }
    let channel = evaluate_channel(compiled.as_ref(), &JsonHost::new(), &ctx);
    let value = match &channel.value {
        None => Json::Null,
        Some(pm_expr::ChannelValue::Number(n)) => encode(&Value::Number(*n)),
        Some(pm_expr::ChannelValue::String(s)) => encode(&Value::String(s.clone())),
        Some(pm_expr::ChannelValue::Bool(b)) => encode(&Value::Bool(*b)),
    };
    fields.push((
        "channel",
        obj(vec![
            ("value", value),
            ("error", channel.error.as_deref().map_or(Json::Null, js)),
        ]),
    ));
    obj(fields)
}

fn mentions_frame(source: &[u16]) -> bool {
    let source = String::from_utf16_lossy(source);
    let b = source.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    source
        .match_indices("frame")
        .any(|(i, _)| (i == 0 || !word(b[i - 1])) && b.get(i + 5).is_none_or(|&c| !word(c)))
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

struct Mulberry32(u32);

impl Mulberry32 {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        t ^ (t >> 14)
    }
}

fn from_bits(hi: u32, lo: u32) -> f64 {
    f64::from_bits((u64::from(hi) << 32) | u64::from(lo))
}

/// Mirrors `sample` in parity/reference.mjs (`6.2831` is wiggle's literal).
#[allow(clippy::approx_constant)]
fn sample(fun: &str, i: usize, rng: &mut Mulberry32) -> (f64, f64) {
    let d: [u32; 6] = std::array::from_fn(|_| rng.next());
    let unit = |k: usize| f64::from(d[k]) / 4294967296.0;
    let range_a = match fun {
        "asin" | "acos" => 1.0,
        "exp" => 745.0,
        "pow" => 20.0,
        _ => 1e3,
    };
    let range_b = match fun {
        "atan2" => 1e3,
        "pow" => 40.0,
        _ => 1.0,
    };
    let mode = i % 5;
    let mut a = match mode {
        0 => from_bits(d[0], d[1]),
        1 => (unit(0) * 2.0 - 1.0) * range_a,
        2 => ((unit(0) * 2001.0).floor() - 1000.0 + (unit(1) * 100.0).floor() / 100.0) * 127.1,
        3 => {
            unit(0) * 60.0 * (unit(1) * 8.0) * 6.2831
                + unit(2) * 17.3
                + (unit(3) * 3.0).floor() * 2.4
        }
        _ => {
            let e = (unit(1) * 80.0).floor() as i64 - 60 + 1023;
            (unit(0) * 2.0 - 1.0) * f64::from_bits((e as u64) << 52)
        }
    };
    let mut b = if mode == 0 {
        from_bits(d[2], d[3])
    } else if mode == 2 && fun == "pow" {
        ((unit(4) * 2.0 - 1.0) * 40.0).floor()
    } else {
        (unit(4) * 2.0 - 1.0) * range_b
    };
    if fun == "log" {
        a = a.abs();
    }
    if (fun == "asin" || fun == "acos") && mode != 0 {
        a = unit(0) * 2.0 - 1.0;
    }
    if fun == "pow" && mode == 1 {
        a = unit(0) * 20.0;
    }
    if fun == "wiggle" {
        a = unit(0) * 60.0;
        b = unit(1) * 8.0;
    }
    (a, b)
}

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
    let random = Expression::compile("random(value)", Dialect::Legacy).unwrap();
    let wiggle = Expression::compile("wiggle(value, 20, 7)", Dialect::Legacy).unwrap();
    let empty = Json::Object(Map::new());
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
            let got = match fun {
                "sin" => jsmath::sin(a),
                "cos" => jsmath::cos(a),
                "tan" => jsmath::tan(a),
                "asin" => jsmath::asin(a),
                "acos" => jsmath::acos(a),
                "atan" => jsmath::atan(a),
                "atan2" => jsmath::atan2(a, b),
                "exp" => jsmath::exp(a),
                "log" => jsmath::log(a),
                "pow" => jsmath::pow(a, b),
                "sqrt" => jsmath::sqrt(a),
                "random" | "wiggle" => {
                    let (expr, t, value) = if fun == "random" {
                        (&random, 0.0, a)
                    } else {
                        (&wiggle, a, b)
                    };
                    let ctx = Context {
                        t,
                        time: 0.0,
                        fps: 30.0,
                        frame: 0.0,
                        value: Value::Number(value),
                        layer: JsonHost::value(&empty),
                        comp: JsonHost::value(&empty),
                        channel: JsStr::from("x"),
                        idx: 0.0,
                    };
                    expr.evaluate(&JsonHost::new(), &ctx)
                        .result
                        .unwrap()
                        .as_number()
                        .unwrap()
                }
                other => panic!("unknown function {other}"),
            };
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
