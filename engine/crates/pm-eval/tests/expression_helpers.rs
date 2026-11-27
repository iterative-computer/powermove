//! Replays `tests/vectors/expression-helper-vectors.json`, written by
//! `tests/vectors/generate-expression-helper-vectors.ts` from main's
//! TypeScript (`anim.ts` `evalProp` with `sample` and `motionPath`,
//! `core/anim/motion-path.ts` `createPathTiming`): a probe layer's
//! `position.x` expression reads other layers through the two helpers; every
//! result must be bit-identical, and every expression error message equal.

use std::sync::Arc;

use pm_eval::{Comp, Evaluator, JsValue};
use pm_model::json::{Value, parse};
use pm_model::{Document, JsString};

#[path = "support/pm_expr_engine.rs"]
mod pm_expr_engine;

fn hex(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().expect("hex"), 16).expect("hex digits"))
}

fn describe(v: &JsValue) -> String {
    match v {
        JsValue::Number(n) => format!("{n:e} ({:016x})", n.to_bits()),
        other => format!("{other:?}"),
    }
}

fn matches(got: &JsValue, want: &Value) -> bool {
    let want = want.as_object().expect("encoded value");
    match got {
        JsValue::Number(n) => want
            .get("n")
            .is_some_and(|w| hex(w).to_bits() == n.to_bits()),
        JsValue::Undefined => want.get("u").is_some(),
        JsValue::Null => want.get("z").is_some(),
        JsValue::Bool(b) => want.get("b").and_then(Value::as_bool) == Some(*b),
        JsValue::String(s) => want.get("s").and_then(Value::as_js_string) == Some(s),
        JsValue::Object(_) => want.get("j").is_some(),
    }
}

#[test]
fn sample_and_motion_path_match_typescript() {
    let vectors = parse(include_str!("vectors/expression-helper-vectors.json")).expect("parse");
    let (mut cases, mut errors, mut failures) = (0usize, 0usize, Vec::new());
    for comp in vectors.get("comps").unwrap().as_array().unwrap() {
        let base = comp.get("doc").unwrap().as_str().unwrap();
        for case in comp.get("cases").unwrap().as_array().unwrap() {
            let source = case.get("source").unwrap().as_str().unwrap();
            let text = base.replace("\"__EXPR__\"", &format!("\"{source}\""));
            let doc = Document::new(Some(parse(&text).expect("document")), 1);
            let cx = Comp::resolve(&doc, "C").expect("composition C");
            let mut ev = Evaluator::new().with_expressions(Arc::new(pm_expr_engine::PmExprEngine));
            let probe = cx
                .layers()
                .iter()
                .find(|l| {
                    pm_eval::node::string(pm_eval::node::member(Some(l), "id"))
                        .map(|s| s.to_string_lossy())
                        .as_deref()
                        == Some("P")
                })
                .expect("probe layer");
            let t = hex(case.get("T").unwrap());
            let got = ev.ev(&cx, Some(probe), &JsString::from("position.x"), t);
            let what = format!("{source} at {t}");
            match got {
                Ok(v) if matches(&v, case.get("out").unwrap()) => {}
                other => failures.push(format!(
                    "{what}: got {}, want {}",
                    other.as_ref().map_or_else(|e| format!("{e:?}"), describe),
                    pm_model::json::to_canonical_string(case.get("out").unwrap())
                )),
            }
            let want_error = case.get("err").and_then(Value::as_str);
            let got_error = ev.expression_error("C", "P", "position.x");
            if want_error.is_some() {
                errors += 1;
            }
            if got_error != want_error {
                failures.push(format!("{what}: error {got_error:?}, want {want_error:?}"));
            }
            cases += 1;
        }
    }
    eprintln!("expression helper vectors: {cases} cases, {errors} with expression errors");
    assert!(cases > 3000, "the vectors were not read");
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures[..failures.len().min(40)].join("\n")
    );
}
