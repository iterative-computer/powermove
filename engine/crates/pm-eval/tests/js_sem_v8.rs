//! `pm_eval::js_sem` against V8 (`tests/vectors/js-sem-vectors.json`, written
//! by `tests/vectors/generate-js-sem-vectors.mjs` under Node).

use pm_eval::JsValue;
use pm_eval::js_sem::{self, math_round, number_to_string_radix, pad_start, parse_int, rem};
use pm_model::JsString;
use pm_model::Value;
use pm_model::json::parse;

fn vectors() -> Value {
    parse(include_str!("vectors/js-sem-vectors.json")).expect("parse")
}

fn f(hex: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(hex.as_str().expect("hex"), 16).expect("hex"))
}

fn decode(v: &Value) -> JsValue {
    let m = v.as_object().expect("encoded");
    if m.get("u").is_some() {
        JsValue::Undefined
    } else if m.get("z").is_some() {
        JsValue::Null
    } else if let Some(n) = m.get("n") {
        JsValue::Number(f(n))
    } else if let Some(s) = m.get("s") {
        JsValue::String(s.as_js_string().unwrap().clone())
    } else if let Some(b) = m.get("b") {
        JsValue::Bool(b.as_bool().unwrap())
    } else {
        JsValue::from(parse(m.get("j").unwrap().as_str().unwrap()).unwrap())
    }
}

fn same(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Number(x), JsValue::Number(y)) => {
            x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan())
        }
        _ => a == b,
    }
}

#[test]
fn operators_match_v8() {
    let v = vectors();
    let mut failures = Vec::new();
    let mut checked = 0;
    for case in v.get("binary").unwrap().as_array().unwrap() {
        let a = decode(case.get("a").unwrap());
        let b = decode(case.get("b").unwrap());
        let n = |k: &str| decode(case.get(k).unwrap());
        let flag = |k: &str| case.get(k).unwrap().as_bool().unwrap();
        let checks: [(&str, JsValue, JsValue); 4] = [
            ("+", js_sem::add(&a, &b), n("add")),
            ("-", JsValue::Number(js_sem::sub(&a, &b)), n("sub")),
            ("*", JsValue::Number(js_sem::mul(&a, &b)), n("mul")),
            ("/", JsValue::Number(js_sem::div(&a, &b)), n("div")),
        ];
        for (op, got, want) in checks {
            checked += 1;
            if !same(&got, &want) {
                failures.push(format!("{a:?} {op} {b:?}: got {got:?}, want {want:?}"));
            }
        }
        let flags = [
            ("==", js_sem::loose_eq(&a, &b), flag("eq")),
            ("===", js_sem::strict_eq(&a, &b), flag("seq")),
            ("<", js_sem::less_than(&a, &b), flag("lt")),
            ("<=", js_sem::less_equal(&a, &b), flag("le")),
            (">", js_sem::greater_than(&a, &b), flag("gt")),
            (">=", js_sem::greater_equal(&a, &b), flag("ge")),
        ];
        for (op, got, want) in flags {
            checked += 1;
            if got != want {
                failures.push(format!("{a:?} {op} {b:?}: got {got}, want {want}"));
            }
        }
    }
    for case in v.get("unary").unwrap().as_array().unwrap() {
        let x = decode(case.get("v").unwrap());
        checked += 4;
        if !same(
            &JsValue::Number(x.to_number()),
            &decode(case.get("number").unwrap()),
        ) {
            failures.push(format!("Number({x:?})"));
        }
        if x.to_js_string().as_str() != case.get("string").unwrap().as_str() {
            failures.push(format!("String({x:?}) = {:?}", x.to_js_string()));
        }
        if x.truthy() != case.get("truthy").unwrap().as_bool().unwrap() {
            failures.push(format!("!!{x:?}"));
        }
        if Some(x.type_of()) != case.get("type").unwrap().as_str() {
            failures.push(format!("typeof {x:?}"));
        }
    }
    eprintln!("{checked} operator results compared");
    assert!(
        failures.is_empty(),
        "{} mismatches:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn math_round_and_remainder_match_v8() {
    let v = vectors();
    let mut n = 0;
    for case in v.get("round").unwrap().as_array().unwrap() {
        let c = case.as_array().unwrap();
        let (x, want) = (f(&c[0]), f(&c[1]));
        let got = math_round(x);
        assert!(
            got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()),
            "Math.round({x:e}) = {got:e}, want {want:e}"
        );
        n += 1;
    }
    for case in v.get("rem").unwrap().as_array().unwrap() {
        let c = case.as_array().unwrap();
        let (a, b, want) = (f(&c[0]), f(&c[1]), f(&c[2]));
        let got = rem(a, b);
        assert!(
            got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()),
            "{a:e} % {b:e} = {got:e}, want {want:e}"
        );
        n += 1;
    }
    eprintln!("{n} Math.round and % results compared");
}

#[test]
fn radix_strings_and_parse_int_match_v8() {
    let v = vectors();
    let mut n = 0;
    for case in v.get("radix").unwrap().as_array().unwrap() {
        let c = case.as_array().unwrap();
        let x = f(&c[0]);
        let radix = c[1].as_f64().unwrap() as u32;
        assert_eq!(
            number_to_string_radix(x, radix),
            c[2].as_str().unwrap(),
            "({x:e}).toString({radix})"
        );
        n += 1;
    }
    for case in v.get("parse").unwrap().as_array().unwrap() {
        let c = case.as_array().unwrap();
        let s = JsString::from(c[0].as_str().unwrap());
        let radix = c[1].as_f64().unwrap() as i32;
        let want = f(&c[2]);
        let got = parse_int(&s, radix);
        assert!(
            got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan()),
            "parseInt({s:?}, {radix}) = {got:e}, want {want:e}"
        );
        n += 1;
    }
    for case in v.get("pad").unwrap().as_array().unwrap() {
        let c = case.as_array().unwrap();
        assert_eq!(
            pad_start(c[0].as_str().unwrap(), c[1].as_f64().unwrap() as usize, '0'),
            c[2].as_str().unwrap()
        );
        n += 1;
    }
    eprintln!("{n} radix, parseInt, and padStart results compared");
}
