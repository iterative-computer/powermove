//! Compares the engine's layer evaluation with the parity corpus's evaluation
//! dumps (`docs/engine/corpus.md`), captured from the legacy app.
//!
//! The corpus lives outside the repository. The test reads the capture named
//! by `PM_ENGINE_CORPUS` (a capture directory), or the newest `v0-*` capture
//! under `~/Library/Application Support/Powermove Engine Corpus/`, and passes
//! with a note when there is none. `PM_CORPUS_ONLY=<prefix>` limits it to
//! projects whose `<group>/<name>` starts with the prefix; `PM_CORPUS_VERBOSE=1`
//! prints every mismatch.
//!
//! Tolerances (plan section 25): numbers within `1e-9 + 1e-9·|b|`, strings and
//! booleans exact; bounds within 1e-6.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_eval::retime::Retiming;
use pm_eval::{Comp, Evaluator, JsValue, Scope};
use pm_model::json::{Map, Value, parse};
use pm_model::{Document, JsString, Node};

use support::pm_expr_engine::PmExprEngine;

fn corpus_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PM_ENGINE_CORPUS") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").ok()?;
    let root = Path::new(&home).join("Library/Application Support/Powermove Engine Corpus");
    let mut captures: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("v0-"))
                && p.join("projects").is_dir()
        })
        .collect();
    captures.sort();
    captures.pop()
}

/// Every project directory (`projects/<group>/<name>`) with a loaded document
/// and evaluation dumps, plus `real/<slug>`.
fn projects(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for top in ["projects", "real"] {
        let Ok(groups) = std::fs::read_dir(dir.join(top)) else {
            continue;
        };
        for group in groups.filter_map(|e| e.ok()) {
            let gp = group.path();
            if gp.join("loaded.json").is_file() {
                out.push((format!("{top}/{}", group.file_name().to_string_lossy()), gp));
                continue;
            }
            let Ok(names) = std::fs::read_dir(&gp) else {
                continue;
            };
            for name in names.filter_map(|e| e.ok()) {
                let p = name.path();
                if p.join("loaded.json").is_file() && p.join("eval").is_dir() {
                    out.push((
                        format!(
                            "{}/{}",
                            group.file_name().to_string_lossy(),
                            name.file_name().to_string_lossy()
                        ),
                        p,
                    ));
                }
            }
        }
    }
    out.sort();
    out
}

#[derive(Default)]
struct Stats {
    compared: BTreeMap<&'static str, usize>,
    failed: BTreeMap<&'static str, usize>,
    max_error: BTreeMap<&'static str, f64>,
    samples: BTreeMap<&'static str, Vec<String>>,
    verbose: bool,
}

impl Stats {
    fn record(&mut self, category: &'static str, ok: bool, what: impl FnOnce() -> String) {
        *self.compared.entry(category).or_default() += 1;
        if !ok {
            *self.failed.entry(category).or_default() += 1;
            let list = self.samples.entry(category).or_default();
            if self.verbose || list.len() < 8 {
                list.push(what());
            }
        }
    }

    fn error(&mut self, category: &'static str, e: f64) {
        let m = self.max_error.entry(category).or_insert(0.0);
        if e > *m {
            *m = e;
        }
    }
}

/// A dumped number: numbers, or `"NaN"` / `"Infinity"` / `"-Infinity"`.
fn dumped_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        Value::String(s) => match s.as_str()? {
            "NaN" => Some(f64::NAN),
            "Infinity" => Some(f64::INFINITY),
            "-Infinity" => Some(f64::NEG_INFINITY),
            _ => None,
        },
        _ => None,
    }
}

fn close(ours: f64, want: f64, abs: f64, rel: f64) -> bool {
    if ours.is_nan() || want.is_nan() {
        return ours.is_nan() && want.is_nan();
    }
    if ours.is_infinite() || want.is_infinite() {
        return ours == want;
    }
    (ours - want).abs() <= abs + rel * want.abs()
}

/// The JSON the capture's `plain()` writes for a value: non-finite numbers as
/// strings, `-0` as 0.
fn plain(v: &Value) -> Value {
    match v {
        Value::Number(n) if !n.is_finite() => Value::String(JsString::from(if n.is_nan() {
            "NaN"
        } else if *n > 0.0 {
            "Infinity"
        } else {
            "-Infinity"
        })),
        Value::Number(n) if *n == 0.0 => Value::Number(0.0),
        Value::Array(items) => Value::Array(items.iter().map(plain).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), plain(v))).collect()),
        other => other.clone(),
    }
}

/// Deep comparison with the numeric tolerance; returns the largest error.
fn same(ours: &Value, want: &Value, abs: f64, rel: f64, err: &mut f64) -> bool {
    match (ours, want) {
        (Value::Number(a), _) if dumped_number(want).is_some() => {
            let b = dumped_number(want).unwrap();
            if a.is_finite() && b.is_finite() {
                let e = (a - b).abs() / (1.0 + b.abs());
                if e > *err {
                    *err = e;
                }
            }
            close(*a, b, abs, rel)
        }
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Null, Value::Null) => true,
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y, abs, rel, err))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(k, x)| b.get_key(k).is_some_and(|y| same(x, y, abs, rel, err)))
        }
        _ => false,
    }
}

fn js_to_value(v: &JsValue) -> Value {
    match v {
        JsValue::Undefined | JsValue::Null => Value::Null,
        JsValue::Bool(b) => Value::Bool(*b),
        JsValue::Number(n) => Value::Number(*n),
        JsValue::String(s) => Value::String(s.clone()),
        JsValue::Object(v) => (**v).clone(),
    }
}

fn numbers(xs: &[f64]) -> Value {
    Value::Array(xs.iter().map(|x| Value::Number(*x)).collect())
}

fn member<'v>(v: &'v Value, key: &str) -> Option<&'v Value> {
    match v {
        Value::Object(m) => m.get_key(&JsString::from(key)),
        _ => None,
    }
}

fn layer_error(state: &Value, label: &str) -> bool {
    member(state, "errors")
        .and_then(|e| member(e, &format!("*{label}")))
        .is_some()
}

struct Ctx<'a> {
    project: &'a str,
    stats: &'a mut Stats,
}

impl Ctx<'_> {
    fn check(
        &mut self,
        category: &'static str,
        what: &str,
        ours: Result<Value, String>,
        want: Option<&Value>,
        threw: bool,
        tol: (f64, f64),
    ) {
        let want = want.unwrap_or(&Value::Null);
        match ours {
            Ok(v) => {
                let v = plain(&v);
                let mut err = 0.0;
                let ok = !threw && same(&v, want, tol.0, tol.1, &mut err);
                self.stats.error(category, err);
                let project = self.project;
                self.stats.record(category, ok, || {
                    format!(
                        "{project} {what}: ours {} want {}{}",
                        pm_model::json::to_canonical_string(&v),
                        pm_model::json::to_canonical_string(want),
                        if threw { " (legacy threw)" } else { "" }
                    )
                });
            }
            Err(e) => {
                let ok = threw || matches!(want, Value::Null);
                let project = self.project;
                self.stats.record(category, ok, || {
                    format!("{project} {what}: ours threw {e}, want {}", pm_model::json::to_canonical_string(want))
                });
            }
        }
    }
}

const NUM: (f64, f64) = (1e-9, 1e-9);

fn compare_comp(
    cx: &mut Ctx<'_>,
    ev: &mut Evaluator,
    doc: &Document,
    comp: &Comp<'_>,
    project: &Comp<'_>,
    state: &Value,
    path: &str,
) {
    let time = dumped_number(member(state, "time").unwrap()).unwrap();
    let layers = comp.layers();
    let Some(Value::Array(dumped)) = member(state, "layers") else {
        return;
    };
    for l in dumped {
        let index = member(l, "index").and_then(dumped_number).unwrap() as usize;
        let Some(layer) = layers.get(index) else {
            cx.stats.record("layer-index", false, || format!("{} {path} missing layer {index}", cx.project));
            continue;
        };
        let id = member(l, "id").map(|v| pm_model::json::to_canonical_string(v)).unwrap_or_default();
        let ty = member(l, "type").and_then(|t| t.as_js_string()).map(|s| s.to_string_lossy()).unwrap_or_default();
        let at = |what: &str| format!("{path} {id} t={time} {what}");
        let mut scope = Scope::new(ev, comp, project);

        // Active state.
        let active = scope.active(layer, time).map(Value::Bool).map_err(|e| e.to_string());
        cx.check("active", &at("active"), active, member(l, "active"), layer_error(l, "active"), NUM);

        // Channels.
        if let Some(Value::Object(channels)) = member(l, "channels") {
            let props = scope.ev.all_props(layer).unwrap_or_default();
            for (key, want) in channels.iter() {
                let in_p = node_has(layer, "p", key);
                let got = if in_p {
                    scope.ev.ev(comp, Some(layer), key, time)
                } else {
                    let prop = props.iter().find(|p| &p.key == key).and_then(|p| p.prop.node());
                    scope.ev.ev_p(comp, Some(layer), prop, time, key)
                };
                let threw = member(l, "errors").and_then(|e| member(e, &key.to_string_lossy())).is_some()
                    && !matches!(want, Value::Number(_) | Value::String(_) | Value::Bool(_));
                cx.check(
                    "channels",
                    &at(&format!("channel {}", key.to_string_lossy())),
                    got.map(|v| js_to_value(&v)).map_err(|e| e.to_string()),
                    Some(want),
                    threw,
                    NUM,
                );
            }
        }

        // Resolved content.
        if member(l, "content").is_some() {
            let got = scope
                .ev
                .resolve_content(comp, layer, time)
                .map(|c| Value::Object(c.iter().map(|(k, v)| (k.clone(), js_to_value(v))).collect::<Map>()))
                .map_err(|e| e.to_string());
            cx.check("content", &at("content"), got, member(l, "content"), layer_error(l, "content"), NUM);
        }

        if ty != "audio" {
            let got = scope.local_matrix(layer, time).map(|m| numbers(&m)).map_err(|e| e.to_string());
            cx.check("localMatrix", &at("localMatrix"), got, member(l, "localMatrix"), layer_error(l, "localMatrix"), NUM);
            let got = scope.world_matrix(layer, time).map(|m| numbers(&m)).map_err(|e| e.to_string());
            cx.check("worldMatrix", &at("worldMatrix"), got, member(l, "worldMatrix"), layer_error(l, "worldMatrix"), NUM);
            let got = scope.world_opacity(layer, time).map(Value::Number).map_err(|e| e.to_string());
            cx.check("worldOpacity", &at("worldOpacity"), got, member(l, "worldOpacity"), layer_error(l, "worldOpacity"), NUM);
        }
        let three_d = member(l, "layerFields").and_then(|f| member(f, "threeD"));
        if matches!(three_d, Some(Value::Bool(true))) {
            let got = scope.plane_matrix(layer, time, false).map(|m| numbers(&m)).map_err(|e| e.to_string());
            cx.check("planeMatrix", &at("planeMatrix"), got, member(l, "planeMatrix"), layer_error(l, "planeMatrix"), NUM);
        }
        if matches!(ty.as_str(), "video" | "precomp" | "audio") {
            let got = scope
                .source_time_with(layer, time, Retiming::Legacy, None)
                .map(Value::Number)
                .map_err(|e| e.to_string());
            cx.check("sourceTime", &at("sourceTime"), got, member(l, "sourceTime"), layer_error(l, "sourceTime"), NUM);
        }
    }

    // Nested compositions at their source times.
    if let Some(Value::Array(nested)) = member(state, "precomps") {
        for entry in nested {
            let comp_id = member(entry, "comp").and_then(|c| c.as_js_string()).map(|s| s.to_string_lossy()).unwrap();
            let Ok(sub) = Comp::resolve(doc, &comp_id) else {
                cx.stats.record("precomp", false, || format!("{} {path}: no composition {comp_id}", cx.project));
                continue;
            };
            let sub_state = member(entry, "state").unwrap();
            let want_time = dumped_number(member(entry, "time").unwrap()).unwrap();
            let got_time = dumped_number(member(sub_state, "time").unwrap()).unwrap();
            assert_eq!(want_time.to_bits(), got_time.to_bits());
            compare_comp(cx, ev, doc, &sub, project, sub_state, &format!("{path}>{comp_id}"));
        }
    }
}

fn node_has(layer: &Node, member: &str, key: &JsString) -> bool {
    match layer.get(member).map(Node::kind) {
        Some(pm_model::NodeKind::Object(map)) => map.get(key.as_bytes()).is_some(),
        _ => false,
    }
}

#[test]
fn corpus_evaluation_matches_legacy() {
    let Some(dir) = corpus_dir() else {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    };
    let only = std::env::var("PM_CORPUS_ONLY").ok();
    let mut stats = Stats {
        verbose: std::env::var("PM_CORPUS_VERBOSE").is_ok(),
        ..Stats::default()
    };
    let mut count = 0;
    for (name, path) in projects(&dir) {
        if only.as_deref().is_some_and(|p| !name.starts_with(p)) {
            continue;
        }
        let loaded = std::fs::read_to_string(path.join("loaded.json")).unwrap();
        let doc = Document::new(Some(parse(&loaded).unwrap()), 1);
        let mut evals: Vec<PathBuf> = std::fs::read_dir(path.join("eval"))
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();
        evals.sort();
        for eval in evals {
            let dump = parse(&std::fs::read_to_string(&eval).unwrap()).unwrap();
            let state = member(&dump, "composition").unwrap();
            let id = member(state, "id").and_then(|c| c.as_js_string()).map(|s| s.to_string_lossy()).unwrap();
            let project = Comp::resolve(&doc, &id).expect("open composition");
            let mut ev = Evaluator::new().with_expressions(Arc::new(PmExprEngine));
            let mut cx = Ctx {
                project: &name,
                stats: &mut stats,
            };
            compare_comp(&mut cx, &mut ev, &doc, &project, &project, state, &id);
            count += 1;
        }
    }
    eprintln!("corpus {}: {count} dumps", dir.display());
    let mut failed = false;
    for (category, n) in &stats.compared {
        let f = stats.failed.get(category).copied().unwrap_or(0);
        eprintln!(
            "  {category:<14} {n:>7} compared, {f:>5} mismatched, max relative error {:.3e}",
            stats.max_error.get(category).copied().unwrap_or(0.0)
        );
        if f > 0 {
            failed = true;
            for s in stats.samples.get(category).into_iter().flatten() {
                eprintln!("      {s}");
            }
        }
    }
    assert!(!failed, "corpus mismatches (see above)");
}
