//! Shared by the parity tests and the wasm parity example: the case encoding
//! `parity/reference.mjs` writes, and the Math sweep's input generator.

#![allow(dead_code)]

use pm_expr::{
    Context, Dialect, Expression, JsStr, JsonHost, JsonRef, Value, evaluate_channel,
    expression_diagnostic_utf16, jsmath, to_js_string,
};
use pm_model::json::{Map, Value as Json};

pub trait JsonExt {
    fn field(&self, key: &str) -> Option<&Json>;
}

impl JsonExt for Json {
    fn field(&self, key: &str) -> Option<&Json> {
        self.as_object().and_then(|m| m.get(key))
    }
}

pub fn context(spec: &Json, frame: f64) -> Context<JsonRef<'_>> {
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

pub fn js(s: &str) -> Json {
    Json::String(s.into())
}

pub fn jstr(s: &JsStr) -> Json {
    Json::String(pm_model::JsString::from_utf16_units(s.units()))
}

pub fn obj(entries: Vec<(&str, Json)>) -> Json {
    let mut m = Map::new();
    for (k, v) in entries {
        m.insert(k, v);
    }
    Json::Object(m)
}

pub fn encode<'a>(value: &Value<JsonRef<'a>>) -> Json {
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
pub fn evaluate_case(source: &[u16], spec: &Json, dialect: Dialect) -> Json {
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
        let untracked = expr.evaluate_untracked(&JsonHost::new(), &ctx);
        match (&out.result, &untracked) {
            (Ok(a), Ok(b)) => assert_eq!(encode(a), encode(b), "untracked result"),
            (Err(a), Err(b)) => assert_eq!(a, b, "untracked exception"),
            _ => panic!("tracked and untracked disagree"),
        }
        if let Some(bound) = expr.bind_numeric(&JsonHost::new(), &ctx) {
            for alternate in [false, true] {
                let mut at = ctx.clone();
                if alternate {
                    at.time = -1.25;
                    at.t = 3.5;
                    at.frame = 72.0;
                    at.value = Value::Number(3.25);
                }
                let original = expr.evaluate_untracked(&JsonHost::new(), &at);
                let prepared = bound.evaluate_untracked(&JsonHost::new(), &at);
                match (&original, &prepared) {
                    (Ok(a), Ok(b)) => assert_eq!(encode(a), encode(b), "bound numeric result"),
                    (Err(a), Err(b)) => assert_eq!(a, b, "bound numeric exception"),
                    _ => panic!("bound numeric plan differs"),
                }
            }
        }
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

pub fn mentions_frame(source: &[u16]) -> bool {
    let source = String::from_utf16_lossy(source);
    let b = source.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    source
        .match_indices("frame")
        .any(|(i, _)| (i == 0 || !word(b[i - 1])) && b.get(i + 5).is_none_or(|&c| !word(c)))
}

// ── Math sweep inputs ───────────────────────────────────────────────────────

pub struct Mulberry32(pub u32);

impl Mulberry32 {
    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        t ^ (t >> 14)
    }
}

pub fn from_bits(hi: u32, lo: u32) -> f64 {
    f64::from_bits((u64::from(hi) << 32) | u64::from(lo))
}

/// Mirrors `sample` in parity/reference.mjs (`6.2831` is wiggle's literal).
#[allow(clippy::approx_constant)]
pub fn sample(fun: &str, i: usize, rng: &mut Mulberry32) -> (f64, f64) {
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

/// The function a sweep entry measures, as the expression language computes it.
pub fn math_apply(fun: &str, a: f64, b: f64) -> f64 {
    match fun {
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
            let (source, t, value) = if fun == "random" {
                ("random(value)", 0.0, a)
            } else {
                ("wiggle(value, 20, 7)", a, b)
            };
            let expr = Expression::compile(source, Dialect::Legacy).expect("compiles");
            let empty = Json::Object(Map::new());
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
                .expect("evaluates")
                .as_number()
                .expect("a number")
        }
        other => panic!("unknown function {other}"),
    }
}

/// The function names of the sweep, in `reference.mjs` order.
pub const MATH_FUNCTIONS: [&str; 13] = [
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "exp", "log", "pow", "sqrt", "random",
    "wiggle",
];
