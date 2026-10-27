//! Replays `tests/vectors/eval-vectors.json`, written by
//! `tests/vectors/generate-eval-vectors.ts` from the TypeScript evaluator.
//!
//! Numbers must agree within the section 25 evaluation tolerance
//! (`|a − b| ≤ 1e-9 + 1e-9·|b|`); the test also counts how many are bit
//! identical and fails if any channel value is not (the port keeps the
//! TypeScript operation order, and channel evaluation has no transcendental
//! functions). Strings, booleans, `null`, `undefined`, JSON values, and thrown
//! error kinds must be equal.
//!
//! `PM_EVAL_VECTORS=/path/to/vectors.json` replays a larger local set.

use std::collections::BTreeMap;
use std::sync::Arc;

use pm_eval::address::{self, NoDefinitions};
use pm_eval::easing::{self, SpringConfig, SpringTables};
use pm_eval::keyframes::{self, PreparedTrack};
use pm_eval::node::{self, Got};
use pm_eval::temporal::{self, EaseKeyframe, Interp, Side, TemporalEase};
use pm_eval::{
    Comp, CompiledExpression, Evaluator, ExprContext, ExprError, ExpressionEngine, JsResult,
    JsValue, PropAddress, Via, js_sem,
};
use pm_model::json::{parse, to_canonical_string};
use pm_model::{Document, JsString, Node, Value};

fn load() -> Value {
    match std::env::var("PM_EVAL_VECTORS") {
        Ok(path) => parse(&std::fs::read_to_string(path).expect("read vectors")).expect("parse"),
        Err(_) => parse(include_str!("vectors/eval-vectors.json")).expect("parse"),
    }
}

fn f(hex: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(hex.as_str().expect("hex"), 16).expect("hex digits"))
}

fn fs(list: &Value) -> Vec<f64> {
    list.as_array().expect("array").iter().map(f).collect()
}

fn s(v: &Value) -> &str {
    v.as_str().expect("string")
}

/// Statistics over compared numbers.
#[derive(Default, Debug)]
struct Stats {
    numbers: usize,
    identical: usize,
    max_abs: f64,
    others: usize,
    failures: Vec<String>,
}

impl Stats {
    fn number(&mut self, what: &str, got: f64, want: f64, require_identical: bool) {
        self.numbers += 1;
        if got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()) {
            self.identical += 1;
            return;
        }
        let diff = (got - want).abs();
        let ok = got == want || diff <= 1e-9 + 1e-9 * want.abs();
        if ok && !require_identical {
            self.max_abs = self.max_abs.max(diff);
            return;
        }
        self.fail(format!(
            "{what}: got {got:e} ({:016x}) want {want:e} ({:016x})",
            got.to_bits(),
            want.to_bits()
        ));
    }

    fn fail(&mut self, message: String) {
        if self.failures.len() < 40 {
            self.failures.push(message);
        } else if self.failures.len() == 40 {
            self.failures.push("…".into());
        }
    }

    fn js(&mut self, what: &str, got: &JsResult<JsValue>, want: &Value) {
        let want_map = want.as_object().expect("encoded value");
        if let Some(kind) = want_map.get("throw") {
            match got {
                Err(e) if e.kind == s(kind) => self.others += 1,
                other => self.fail(format!("{what}: got {other:?}, want throw {}", s(kind))),
            }
            return;
        }
        let Ok(got) = got else {
            self.fail(format!(
                "{what}: threw {:?}, want {}",
                got,
                to_canonical_string(want)
            ));
            return;
        };
        if let Some(n) = want_map.get("n") {
            match got {
                JsValue::Number(g) => self.number(what, *g, f(n), true),
                other => self.fail(format!("{what}: got {other:?}, want number {:e}", f(n))),
            }
            return;
        }
        self.others += 1;
        let ok = match got {
            JsValue::Undefined => want_map.get("u").is_some(),
            JsValue::Null => want_map.get("z").is_some(),
            JsValue::Bool(b) => want_map.get("b").and_then(Value::as_bool) == Some(*b),
            JsValue::String(text) => want_map.get("s").and_then(Value::as_js_string) == Some(text),
            JsValue::Number(_) => false,
            JsValue::Object(v) => want_map.get("j").is_some_and(|j| {
                to_canonical_string(&parse(s(j)).expect("json")) == to_canonical_string(&**v)
            }),
        };
        if !ok {
            self.others -= 1;
            self.fail(format!(
                "{what}: got {got:?}, want {}",
                to_canonical_string(want)
            ));
        }
    }

    fn check(&self, section: &str) {
        eprintln!(
            "{section}: {} numbers ({} bit-identical, max |diff| {:e}), {} other values",
            self.numbers, self.identical, self.max_abs, self.others
        );
        assert!(
            self.failures.is_empty(),
            "{section} mismatches:\n{}",
            self.failures.join("\n")
        );
    }
}

#[test]
fn easing_matches_typescript() {
    let vectors = load();
    let mut stats = Stats::default();
    for case in vectors.get("easing").unwrap().as_array().unwrap() {
        let c = fs(case.get("curve").unwrap());
        let curve = easing::bezier(c[0], c[1], c[2], c[3]);
        if let Some(Value::String(name)) = case.get("name") {
            let p = easing::preset(name.as_str().unwrap());
            assert_eq!(
                p.map(f64::to_bits),
                [c[0], c[1], c[2], c[3]].map(f64::to_bits),
                "preset {name}"
            );
        }
        for (x, y) in fs(case.get("xs").unwrap())
            .into_iter()
            .zip(fs(case.get("ys").unwrap()))
        {
            stats.number(&format!("bezier {c:?} at {x}"), curve.eval(x), y, true);
        }
    }
    for case in vectors.get("names").unwrap().as_array().unwrap() {
        let eo = fs(case.get("eo").unwrap());
        let ei = fs(case.get("ei").unwrap());
        assert_eq!(
            easing::name_of([eo[0], eo[1]], [ei[0], ei[1]]),
            s(case.get("name").unwrap())
        );
        stats.others += 1;
    }
    stats.check("easing");
}

#[test]
fn springs_match_typescript() {
    let vectors = load();
    assert_eq!(
        vectors.get("springTableLength").unwrap().as_f64().unwrap() as usize,
        easing::spring_table_len()
    );
    let mut stats = Stats::default();
    let mut tables = SpringTables::new();
    for case in vectors.get("springs").unwrap().as_array().unwrap() {
        let cfg_text = s(case.get("cfg").unwrap());
        let cfg_node = Node::from_value(parse(cfg_text).unwrap(), 0);
        let cfg = SpringConfig::from_node(Some(&cfg_node));
        for (t, y) in fs(case.get("ts").unwrap())
            .into_iter()
            .zip(fs(case.get("ys").unwrap()))
        {
            stats.number(
                &format!("spring {cfg_text} at {t}"),
                tables.spring(t, cfg),
                y,
                true,
            );
        }
        stats.number(
            &format!("springDuration {cfg_text}"),
            easing::spring_duration(cfg),
            f(case.get("duration").unwrap()),
            true,
        );
    }
    stats.check("springs");
}

fn interp(v: &Value) -> Interp {
    Interp::from_name(v.as_str().map(str::as_bytes))
}

fn ease(v: &Value) -> TemporalEase {
    TemporalEase {
        speed: f(v.get("speed").unwrap()),
        influence: f(v.get("influence").unwrap()),
    }
}

fn typed_key(v: &Value) -> EaseKeyframe {
    let value = match v.get("v").unwrap() {
        Value::Null => None,
        hex => Some(f(hex)).filter(|x| x.is_finite()),
    };
    EaseKeyframe {
        t: f(v.get("t").unwrap()),
        v: value,
        in_interp: interp(v.get("inInterp").unwrap()),
        out_interp: interp(v.get("outInterp").unwrap()),
        in_ease: ease(v.get("inEase").unwrap()),
        out_ease: ease(v.get("outEase").unwrap()),
        auto_bezier: v.get("autoBezier").unwrap().as_bool().unwrap(),
        continuous: v.get("continuous").unwrap().as_bool().unwrap(),
    }
}

#[test]
fn temporal_ease_matches_typescript() {
    let vectors = load();
    let temporal_v = vectors.get("temporal").unwrap();
    let mut stats = Stats::default();
    for case in temporal_v.get("solve").unwrap().as_array().unwrap() {
        let v = fs(case);
        stats.number(
            &format!("solveCurveParam{v:?}"),
            temporal::solve_curve_param(v[0], v[1], v[2]),
            v[3],
            true,
        );
    }
    for case in temporal_v.get("segments").unwrap().as_array().unwrap() {
        let a = typed_key(case.get("a").unwrap());
        let b = typed_key(case.get("b").unwrap());
        let times = fs(case.get("times").unwrap());
        for ((t, value), param) in times
            .iter()
            .zip(fs(case.get("values").unwrap()))
            .zip(fs(case.get("params").unwrap()))
        {
            stats.number(
                "valueAtTime",
                temporal::value_at_time(&a, &b, *t),
                value,
                true,
            );
            stats.number(
                "segmentParamAtTime",
                temporal::segment_param_at_time(&a, &b, *t),
                param,
                true,
            );
        }
        match (
            temporal::segment_unit_curve(&a, &b),
            case.get("unit").unwrap(),
        ) {
            (None, Value::Null) => stats.others += 1,
            (Some(got), want @ Value::Array(_)) => {
                for (g, w) in got.iter().zip(fs(want)) {
                    stats.number("segmentUnitCurve", *g, w, true);
                }
            }
            (got, want) => stats.fail(format!(
                "segmentUnitCurve {got:?} vs {}",
                to_canonical_string(want)
            )),
        }
        let p = temporal::segment_control_points(&a, &b);
        let got = [
            p.p0.t, p.p0.v, p.p1.t, p.p1.v, p.p2.t, p.p2.v, p.p3.t, p.p3.v,
        ];
        for (g, w) in got.iter().zip(fs(case.get("points").unwrap())) {
            stats.number("segmentControlPoints", *g, w, true);
        }
    }
    for case in temporal_v.get("handles").unwrap().as_array().unwrap() {
        let h = fs(case.get("handle").unwrap());
        let dt = f(case.get("dt").unwrap());
        let dv = f(case.get("dv").unwrap());
        let side = if s(case.get("side").unwrap()) == "in" {
            Side::In
        } else {
            Side::Out
        };
        let e = temporal::ease_from_unit_handle([h[0], h[1]], dt, dv, side);
        let want = fs(case.get("ease").unwrap());
        stats.number("easeFromUnitHandle speed", e.speed, want[0], true);
        stats.number("easeFromUnitHandle influence", e.influence, want[1], true);
        match (
            temporal::unit_handle_from_ease(e, dt, dv, side),
            case.get("back").unwrap(),
        ) {
            (None, Value::Null) => stats.others += 1,
            (Some(got), want @ Value::Array(_)) => {
                for (g, w) in got.iter().zip(fs(want)) {
                    stats.number("unitHandleFromEase", *g, w, true);
                }
            }
            (got, want) => stats.fail(format!(
                "unitHandleFromEase {got:?} vs {}",
                to_canonical_string(want)
            )),
        }
    }
    for case in temporal_v.get("auto").unwrap().as_array().unwrap() {
        let keys: Vec<EaseKeyframe> = case
            .get("keys")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(typed_key)
            .collect();
        stats.number(
            "autoBezierSpeed",
            temporal::auto_bezier_speed(Some(&keys[0]), &keys[1], Some(&keys[2])),
            f(case.get("speed").unwrap()),
            true,
        );
    }
    stats.check("temporal-ease");
}

#[test]
fn keyframe_tracks_match_typescript() {
    let vectors = load();
    let mut stats = Stats::default();
    let mut springs = SpringTables::new();
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for case in vectors.get("tracks").unwrap().as_array().unwrap() {
        let kind = s(case.get("kind").unwrap()).to_owned();
        let text = s(case.get("kf").unwrap());
        let kf = Node::from_value(parse(text).unwrap(), 0);
        let color = case.get("color").unwrap().as_bool().unwrap();
        let prepared = PreparedTrack::prepare(Some(&kf));
        let times = fs(case.get("times").unwrap());
        let outs = case.get("out").unwrap().as_array().unwrap();
        for (t, want) in times.iter().zip(outs) {
            let what = format!("{kind} {text} color={color} at {t}");
            stats.js(
                &what,
                &keyframes::evaluate(Some(&kf), *t, color, &mut springs),
                want,
            );
            stats.js(
                &format!("prepared {what}"),
                &prepared.value(Some(&kf), *t, color, &mut springs),
                want,
            );
            if let (Some(track), Some(n)) = (prepared.temporal(), want.get("n"))
                && track.is_numeric()
                && t.is_finite()
            {
                stats.number(
                    &format!("value_number {what}"),
                    track.value_number(*t),
                    f(n),
                    true,
                );
            }
        }
        *by_kind.entry(kind).or_default() += 1;
    }
    eprintln!("tracks by kind: {by_kind:?}");
    stats.check("keyframes");
}

/// The expression sources the generator uses, evaluated as
/// `legacy/core/expression.ts` evaluates them. The real interpreter is
/// `pm-expr`'s; this mock exercises `pm-eval`'s side of the interface.
struct MockEngine;

struct Mock(&'static str);

impl ExpressionEngine for MockEngine {
    fn compile(&self, source: &JsString) -> Option<Arc<dyn CompiledExpression>> {
        const KNOWN: [&str; 14] = [
            "value * 2",
            "t",
            "time",
            "fps",
            "idx",
            "param('speed')",
            "param('missing')",
            "'label'",
            "1/0",
            "value > 0",
            "ch",
            "value + 'x'",
            "layer.from",
            "comp.dur",
        ];
        KNOWN
            .iter()
            .find(|k| source.as_str() == Some(k))
            .map(|k| Arc::new(Mock(k)) as Arc<dyn CompiledExpression>)
    }
}

impl CompiledExpression for Mock {
    fn needs_idx(&self) -> bool {
        self.0 == "idx"
    }

    fn evaluate(&self, cx: &ExprContext<'_>) -> Result<JsValue, ExprError> {
        let thrown = |e: pm_eval::JsThrow| ExprError {
            message: Some(e.message),
        };
        Ok(match self.0 {
            "value * 2" => JsValue::Number(cx.value.to_number() * 2.0),
            "t" => JsValue::Number(cx.t),
            "time" => JsValue::Number(cx.time),
            "fps" => cx.fps.clone(),
            "idx" => JsValue::Number(cx.idx),
            "param('speed')" => cx.param(&JsString::from("speed")).map_err(thrown)?,
            "param('missing')" => cx.param(&JsString::from("missing")).map_err(thrown)?,
            "'label'" => JsValue::from("label"),
            "1/0" => JsValue::Number(1.0 / 0.0),
            "value > 0" => JsValue::Bool(cx.value.to_number() > 0.0),
            "ch" => JsValue::String(cx.channel.clone()),
            "value + 'x'" => js_sem::add(
                &JsValue::String(cx.value.to_js_string()),
                &JsValue::from("x"),
            ),
            // member(): own properties of objects only.
            "layer.from" => pm_eval::node::js_value_opt(pm_eval::node::member(cx.layer, "from")),
            "comp.dur" => pm_eval::node::js_value_opt(cx.comp.member("dur")),
            _ => unreachable!(),
        })
    }
}

fn enc_got(got: &Got<'_>) -> JsValue {
    got.to_js()
}

fn run_comp(doc: &Document, case_list: &[Value], stats: &mut Stats, form: &str) {
    let cx = Comp::resolve(doc, "C").expect("composition C");
    let mut ev = Evaluator::new().with_expressions(Arc::new(MockEngine));
    let layers = cx.layers();
    let first_of_id = |index: usize| {
        let id = pm_eval::node::string(pm_eval::node::member(Some(&layers[index]), "id"));
        id.and_then(|id| {
            layers.iter().position(|l| {
                pm_eval::node::string(pm_eval::node::member(Some(l), "id")) == Some(id)
            })
        }) == Some(index)
    };
    for case in case_list {
        let op = s(case.get("op").unwrap());
        let index = case.get("layer").unwrap().as_f64().unwrap() as usize;
        let layer = &layers[index];
        let layer_id = pm_eval::node::string(pm_eval::node::member(Some(layer), "id"))
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let want = case.get("out").unwrap();
        let what = |extra: &str| format!("[{form}] {op} layer {index} ({layer_id}) {extra}");
        match op {
            "all" => {
                let got = address::all_props(layer, &NoDefinitions);
                match (got, want) {
                    (Err(e), Value::Object(w)) => {
                        if w.get("throw").and_then(Value::as_str) == Some(e.kind) {
                            stats.others += 1;
                        } else {
                            stats.fail(what(&format!(
                                "threw {e:?}, want {}",
                                to_canonical_string(want)
                            )));
                        }
                    }
                    (Ok(entries), Value::Array(w)) => {
                        let got_keys: Vec<String> =
                            entries.iter().map(|e| e.key.to_string_lossy()).collect();
                        let want_keys: Vec<String> = w
                            .iter()
                            .map(|e| s(e.get("address").unwrap()).to_owned())
                            .collect();
                        if got_keys != want_keys {
                            stats.fail(what(&format!("keys {got_keys:?}\n want {want_keys:?}")));
                            continue;
                        }
                        for (e, wv) in entries.iter().zip(w) {
                            let k = e.key.to_string_lossy();
                            stats.js(
                                &what(&format!("{k} label")),
                                &Ok(e.label.clone()),
                                wv.get("label").unwrap(),
                            );
                            stats.js(
                                &what(&format!("{k} group")),
                                &Ok(e.group.clone()),
                                wv.get("group").unwrap(),
                            );
                            stats.js(
                                &what(&format!("{k} prop")),
                                &Ok(enc_got(&e.prop)),
                                wv.get("prop").unwrap(),
                            );
                        }
                    }
                    (got, _) => stats.fail(what(&format!(
                        "got {got:?}, want {}",
                        to_canonical_string(want)
                    ))),
                }
            }
            "find" => {
                let key = JsString::from(s(case.get("address").unwrap()));
                let got = address::find_prop(layer, &key).map(|g| {
                    if g.is_nullish() {
                        JsValue::Null
                    } else {
                        g.to_js()
                    }
                });
                stats.js(&what(&key.to_string_lossy()), &got, want);
            }
            "ev" | "evP" => {
                let key = JsString::from(s(case.get("address").unwrap()));
                let t = f(case.get("T").unwrap());
                // The channel object the call reads (`L.p[key]` for ev, the
                // found property for evP); the generator reports
                // `PM.expressionErrors.get(prop)` only when it is an object.
                let read: Option<&pm_model::Node> = if op == "ev" {
                    match node::get(layer, b"p") {
                        Got::Node(p) => node::get(p, key.as_bytes()).node(),
                        _ => None,
                    }
                } else {
                    address::find_prop(layer, &key)
                        .expect("generator skipped throwing finds")
                        .node()
                };
                let got = if op == "ev" {
                    ev.ev(&cx, Some(layer), &key, t)
                } else {
                    ev.ev_p(&cx, Some(layer), read, t, &key)
                };
                stats.js(
                    &what(&format!("{} at {t}", key.to_string_lossy())),
                    &got,
                    want,
                );
                let reaches_object = read.is_some_and(|p| {
                    node::truthy(Some(p))
                        && matches!(
                            p.kind(),
                            pm_model::NodeKind::Object(_) | pm_model::NodeKind::Array(_)
                        )
                });
                // Errors are kept per (composition, layer id, key), not per
                // channel object (allowed difference 7): a call that reads no
                // channel object leaves whatever another object under the
                // same key recorded, where JavaScript has none to look up.
                let err = if reaches_object {
                    ev.expression_error("C", &layer_id, &key.to_string_lossy())
                        .map(str::to_owned)
                } else {
                    None
                };
                let want_err = case.get("err").unwrap().as_str().map(str::to_owned);
                if err != want_err {
                    stats.fail(what(&format!(
                        "{} at {t}: error {err:?}, want {want_err:?}",
                        key.to_string_lossy()
                    )));
                }
                // The cached path must agree, evaluated twice (memo).
                if first_of_id(index) && !layer_id.is_empty() {
                    let address = PropAddress {
                        comp: JsString::from("C"),
                        layer: JsString::from(layer_id.as_str()),
                        key: key.clone(),
                        via: if op == "ev" { Via::Channel } else { Via::Find },
                    };
                    let id = ev.intern(address);
                    for round in 0..2 {
                        let got = ev.ev_id(doc, id, t);
                        stats.js(
                            &what(&format!("ev_id#{round} {} at {t}", key.to_string_lossy())),
                            &got,
                            want,
                        );
                    }
                }
            }
            "active" => {
                let t = f(case.get("T").unwrap());
                let got = ev.active(&cx, Some(layer), t).map(JsValue::Bool);
                stats.js(&what(&format!("at {t}")), &got, want);
            }
            "content" => {
                let t = f(case.get("T").unwrap());
                match (ev.resolve_content(&cx, layer, t), want) {
                    (Err(e), Value::Object(w)) if w.get("throw").is_some() => {
                        if w.get("throw").and_then(Value::as_str) != Some(e.kind) {
                            stats.fail(what(&format!(
                                "threw {e:?}, want {}",
                                to_canonical_string(want)
                            )));
                        }
                    }
                    (Ok(content), Value::Object(w)) => {
                        let got_keys: Vec<String> =
                            content.keys().map(|k| k.to_string_lossy()).collect();
                        let mut want_keys: Vec<String> =
                            w.keys().map(|k| k.to_string_lossy()).collect();
                        want_keys.sort();
                        let mut sorted = got_keys.clone();
                        sorted.sort();
                        if sorted != want_keys {
                            stats
                                .fail(what(&format!("content keys {sorted:?} want {want_keys:?}")));
                            continue;
                        }
                        for (k, v) in &content {
                            let wv = w.get_key(k).unwrap();
                            stats.js(
                                &what(&format!("content {} at {t}", k.to_string_lossy())),
                                &Ok(v.clone()),
                                wv,
                            );
                        }
                    }
                    (got, _) => stats.fail(what(&format!(
                        "got {got:?}, want {}",
                        to_canonical_string(want)
                    ))),
                }
            }
            other => panic!("unknown op {other}"),
        }
    }
}

#[test]
fn layers_match_typescript() {
    let vectors = load();
    let mut stats = Stats::default();
    for comp in vectors.get("comps").unwrap().as_array().unwrap() {
        let root = parse(s(comp.get("doc").unwrap())).unwrap();
        let cases = comp.get("cases").unwrap().as_array().unwrap();
        // The stored form, as the renderer holds it.
        let doc = Document::new(Some(root.clone()), 1);
        run_comp(&doc, cases, &mut stats, "stored");
        // The canonical form the engine mirrors.
        let doc = Document::new(Some(pm_model::canonical::canonical_project(root)), 1);
        run_comp(&doc, cases, &mut stats, "canonical");
    }
    stats.check("layers");
}
