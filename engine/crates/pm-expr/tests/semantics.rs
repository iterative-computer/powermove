//! Targeted behaviour: dependencies, the native dialect, channel outcomes,
//! errors, and diagnostics. Bit-for-bit parity with V8 is in `parity.rs`.

use pm_expr::{
    ChannelValue, Context, ContextReads, Dialect, EvalError, Expression, FieldRead, JsStr,
    JsonHost, JsonRef, NOT_FINITE_MESSAGE, Root, SYNTAX_INVALID_MESSAGE, Step, Value,
    evaluate_channel, expression_diagnostic, expression_names,
};
use pm_model::json::{Value as Json, parse};

struct Doc {
    layer: Json,
    comp: Json,
}

fn doc() -> Doc {
    Doc {
        layer: parse(
            r#"{"name":"Title","from":1,"p":{"x":{"v":4}},"tags":["a","b"],"toString":3}"#,
        )
        .unwrap(),
        comp: parse(r#"{"w":1920,"layers":[{"id":"a"},{"id":"b"}],"params":{"gain":{"value":3}}}"#)
            .unwrap(),
    }
}

fn ctx(doc: &Doc) -> Context<JsonRef<'_>> {
    Context {
        t: 0.5,
        time: 1.5,
        fps: 30.0,
        frame: 45.0,
        value: Value::Number(10.0),
        layer: JsonHost::value(&doc.layer),
        comp: JsonHost::value(&doc.comp),
        channel: JsStr::from("position.x"),
        idx: 1.0,
    }
}

fn eval(source: &str, dialect: Dialect) -> (Result<Value<()>, EvalError>, pm_expr::Dependencies) {
    let d = doc();
    let expr = Expression::compile(source, dialect).expect("compiles");
    let out = expr.evaluate(&JsonHost::new(), &ctx(&d));
    (out.result.map(|v| v.map_doc(|_| ())), out.deps)
}

fn key(s: &str) -> Step {
    Step::Key(JsStr::from(s))
}

fn read(root: Root, path: Vec<Step>) -> FieldRead {
    FieldRead { root, path }
}

fn number(r: &Result<Value<()>, EvalError>) -> f64 {
    r.as_ref().unwrap().as_number().unwrap()
}

#[test]
fn records_member_paths() {
    let (r, deps) = eval("layer.p.x.v * 2", Dialect::Native);
    assert_eq!(number(&r), 8.0);
    assert_eq!(
        deps.fields,
        vec![
            read(Root::Layer, vec![]),
            read(Root::Layer, vec![key("p")]),
            read(Root::Layer, vec![key("p"), key("x")]),
            read(Root::Layer, vec![key("p"), key("x"), key("v")]),
        ]
    );
    assert_eq!(deps.context, ContextReads::empty());
    assert!(!deps.depends_on_time());
    assert!(deps.affected_by(Root::Layer, &[key("p"), key("x"), key("v")]));
    assert!(deps.affected_by(Root::Layer, &[key("p")]));
    assert!(!deps.affected_by(Root::Layer, &[key("name")]));
    assert!(!deps.affected_by(Root::Comp, &[]));
}

#[test]
fn records_context_reads() {
    let (_, deps) = eval(
        "t + time + T + fps + ch.length + idx + value",
        Dialect::Native,
    );
    let all = ContextReads::LOCAL_TIME
        | ContextReads::TIME
        | ContextReads::FPS
        | ContextReads::CHANNEL
        | ContextReads::INDEX
        | ContextReads::VALUE;
    assert_eq!(deps.context, all);
    let (_, deps) = eval("wiggle(2, 10)", Dialect::Native);
    assert_eq!(deps.context, ContextReads::LOCAL_TIME);
    assert!(deps.depends_on_time());
    let (_, deps) = eval("random(3) + PI + Math.E", Dialect::Native);
    assert_eq!(deps.context, ContextReads::empty());
    assert!(deps.fields.is_empty());
}

#[test]
fn records_only_the_branch_taken() {
    let (_, deps) = eval("t > 100 ? layer.name : comp.w", Dialect::Native);
    assert!(deps.fields.contains(&read(Root::Comp, vec![key("w")])));
    assert!(!deps.fields.iter().any(|f| f.root == Root::Layer));
    let (_, deps) = eval("0 && layer.name", Dialect::Native);
    assert!(deps.fields.is_empty());
}

#[test]
fn records_params_array_reads_and_conversions() {
    let (r, deps) = eval("param('gain')", Dialect::Native);
    assert_eq!(number(&r), 3.0);
    for path in [
        vec![key("params")],
        vec![key("params"), key("gain")],
        vec![key("params"), key("gain"), key("value")],
    ] {
        assert!(deps.fields.contains(&read(Root::Comp, path)));
    }
    let (r, deps) = eval("comp.layers.length + layer.tags[1]", Dialect::Native);
    assert!(matches!(r, Ok(Value::String(ref s)) if s.eq_str("2b")));
    assert!(
        deps.fields
            .contains(&read(Root::Comp, vec![key("layers"), Step::Length]))
    );
    assert!(
        deps.fields
            .contains(&read(Root::Layer, vec![key("tags"), Step::Index(1)]))
    );
    // Joining an array reads its length and every element; converting an
    // object probes for an own toString.
    let (_, deps) = eval("comp.layers + ''", Dialect::Native);
    for path in [
        vec![key("layers"), Step::Length],
        vec![key("layers"), Step::Index(0)],
        vec![key("layers"), Step::Index(1), key("toString")],
    ] {
        assert!(deps.fields.contains(&read(Root::Comp, path)), "{deps:?}");
    }
}

#[test]
fn layer_with_own_to_string_throws_on_conversion() {
    let (r, _) = eval("layer + 1", Dialect::Native);
    assert_eq!(r.unwrap_err(), EvalError::ToPrimitive);
    assert_eq!(
        EvalError::ToPrimitive.message(),
        "Cannot convert object to primitive value"
    );
    let (r, _) = eval("layer.toString", Dialect::Native);
    assert_eq!(number(&r), 3.0);
}

#[test]
fn object_identity_follows_the_host() {
    let d = Doc {
        layer: Json::Null,
        comp: parse(r#"{"layers":[{"id":"a"},{"id":"b"}]}"#).unwrap(),
    };
    let layers = match &d.comp {
        Json::Object(m) => m.get("layers").unwrap(),
        _ => unreachable!(),
    };
    let second = match layers {
        Json::Array(items) => &items[1],
        _ => unreachable!(),
    };
    let mut c = ctx(&d);
    c.layer = JsonHost::value(second);
    let expr = Expression::compile(
        "comp.layers[idx] === layer && comp.layers[0] !== layer",
        Dialect::Native,
    )
    .unwrap();
    let out = expr.evaluate(&JsonHost::new(), &c);
    assert!(matches!(out.result, Ok(Value::Bool(true))));
}

#[test]
fn frame_exists_only_in_the_native_dialect() {
    let (r, deps) = eval("frame + 1", Dialect::Native);
    assert_eq!(number(&r), 46.0);
    assert_eq!(deps.context, ContextReads::FRAME);
    assert!(deps.depends_on_time());
    let (r, _) = eval("frame + 1", Dialect::Legacy);
    assert!(number(&r).is_nan());
    assert_eq!(expression_diagnostic("frame", Dialect::Native), None);
    assert_eq!(
        expression_diagnostic("frame", Dialect::Legacy).as_deref(),
        Some("Unknown name “frame”. Use time, value, or a supported function.")
    );
    let legacy = expression_names(Dialect::Legacy);
    let native = expression_names(Dialect::Native);
    assert_eq!(legacy.len() + 1, native.len());
    assert_eq!(&legacy[..4], ["time", "t", "T", "fps"]);
    assert_eq!(&native[..5], ["time", "t", "T", "fps", "frame"]);
    assert_eq!(legacy.last(), Some(&"tan"));
}

#[test]
fn channel_outcomes_follow_eval_prop() {
    let d = doc();
    let c = ctx(&d);
    let host = JsonHost::new();
    let run =
        |s: &str| evaluate_channel(Expression::compile(s, Dialect::Native).as_ref(), &host, &c);
    assert_eq!(run("value * 2").value, Some(ChannelValue::Number(20.0)));
    assert_eq!(
        run("'#ff0000'").value,
        Some(ChannelValue::String(JsStr::from("#ff0000")))
    );
    assert_eq!(run("t > 0").value, Some(ChannelValue::Bool(true)));
    for kept in ["1 / 0", "layer", "undefined", "null", "0 / 0"] {
        let out = run(kept);
        assert_eq!(out.value, None, "{kept}");
        assert_eq!(out.error.as_deref(), Some(NOT_FINITE_MESSAGE), "{kept}");
    }
    let out = run("layer + 1");
    assert_eq!(
        out.error.as_deref(),
        Some("Cannot convert object to primitive value")
    );
    let out = run("1 +");
    assert_eq!(out.error.as_deref(), Some(SYNTAX_INVALID_MESSAGE));
}

#[test]
fn compile_rejects_empty_and_long_sources() {
    assert!(Expression::compile("", Dialect::Native).is_none());
    let string = |n: usize| format!("'{}'", "a".repeat(n - 2));
    assert!(Expression::compile(&string(2000), Dialect::Native).is_some());
    assert!(Expression::compile(&string(2001), Dialect::Native).is_none());
    // A 2,000-digit number is a valid length but not a finite number.
    assert!(Expression::compile(&"1".repeat(2000), Dialect::Native).is_none());
    // The limit counts UTF-16 code units, as JavaScript's length does.
    let emoji = format!("'{}'", "😀".repeat(999));
    assert_eq!(emoji.encode_utf16().count(), 2000);
    assert!(Expression::compile(&emoji, Dialect::Native).is_some());
    assert!(Expression::compile(&format!("'{}'", "😀".repeat(1000)), Dialect::Native).is_none());
    assert_eq!(
        expression_diagnostic(&"x".repeat(2001), Dialect::Native).as_deref(),
        Some("Use an expression of at most 2000 characters.")
    );
}

#[test]
fn diagnostics_match_expression_ts() {
    let d = |s: &str| expression_diagnostic(s, Dialect::Legacy);
    assert_eq!(d("value + Math.sin(t) * 10"), None);
    assert_eq!(d("layer.anything['goes']"), None);
    assert_eq!(d("'unknown names in strings are fine'"), None);
    assert_eq!(
        d("foo + 1").as_deref(),
        Some("Unknown name “foo”. Use time, value, or a supported function.")
    );
    assert_eq!(
        d("t(1)").as_deref(),
        Some("“t” is not a supported function.")
    );
    assert_eq!(
        d("Math.random()").as_deref(),
        Some("Only the documented Math functions can be called.")
    );
    assert_eq!(
        d("layer.name(1)").as_deref(),
        Some("Only the documented Math functions can be called.")
    );
    assert_eq!(d("(1)(2)"), None);
    assert_eq!(d("1 +").as_deref(), Some("Expected a value at 3"));
    assert_eq!(d("1 @ 2").as_deref(), Some("Unsupported token at 2"));
    assert_eq!(d("'abc").as_deref(), Some("Unterminated string at 0"));
    assert_eq!(d("1e999").as_deref(), Some("Invalid number at 0"));
    assert_eq!(d("a.(").as_deref(), Some("Expected a property at 2"));
    // `.1` is a number token, so `a.1` is two values in a row.
    assert_eq!(d("a.1").as_deref(), Some("Unexpected token at 1"));
    assert_eq!(d("(1").as_deref(), Some("Expected ) at 2"));
    assert_eq!(d("1 2").as_deref(), Some("Unexpected token at 2"));
}

#[test]
fn quirks_of_the_original_are_kept() {
    let n = |s: &str| number(&eval(s, Dialect::Legacy).0);
    // String tokens compare by value with operators and punctuation.
    assert_eq!(n("1 \"+\" 2"), 3.0);
    assert_eq!(n("sin \"(\" 0 \")\""), 0.0);
    // Object.prototype names act as inert operators.
    assert!(matches!(
        eval("1 constructor 2", Dialect::Legacy).0,
        Ok(Value::Undefined)
    ));
    assert_eq!(n("1 toString 2 ? 3 : 4"), 4.0);
    // ...and as callees through the inherited lookup.
    assert_eq!(n("constructor(5) + 1"), 6.0);
    assert!(
        matches!(eval("toString()", Dialect::Legacy).0, Ok(Value::String(ref s)) if s.eq_str("[object Undefined]"))
    );
    assert_eq!(
        eval("valueOf()", Dialect::Legacy).0.unwrap_err(),
        EvalError::NullishToObject
    );
    assert_eq!(
        eval("__proto__(1)", Dialect::Legacy)
            .0
            .unwrap_err()
            .message(),
        "math is not a function"
    );
    // Helpers are `{ callable: name }` objects.
    assert!(
        matches!(eval("clamp.callable", Dialect::Legacy).0, Ok(Value::String(ref s)) if s.eq_str("clamp"))
    );
    // Loose equality and coercion.
    assert!(matches!(
        eval(
            "'2' == 2 && null == undefined && !(null == 0)",
            Dialect::Legacy
        )
        .0,
        Ok(Value::Bool(true))
    ));
    assert_eq!(n("comp.layers.length * '3'"), 6.0);
    assert_eq!(n("-7 % 3"), -1.0);
    assert_eq!(n("Math.round(-2.5)"), -2.0);
}
