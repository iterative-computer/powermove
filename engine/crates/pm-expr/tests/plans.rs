//! The typed numeric plan and the general binding against the bytecode
//! evaluator, bit for bit.
//!
//! `parity.rs` proves the bytecode evaluator equals V8 on saved Electron
//! answers. This differential fuzz proves the numeric plan (which answers most
//! channel expressions) and the general binding (partial evaluation of every
//! other source) equal the bytecode evaluator: on generated sources
//! over the subset it compiles (ternaries, comparisons, `!`, `&&`/`||`, the
//! helpers, `Math`) mixed with constructs it must decline (strings, `null`,
//! non-numeric document fields, helpers answering `undefined`), under contexts
//! with signed zeros, NaN, infinities, booleans, strings and objects. Every
//! answer path is compared: untracked evaluation, the document-bound plan, and
//! the scalar plan at other times and values; a general binding also against
//! other documents whenever its own validation accepts them. The test counts
//! how often each path answered and how often validation rejected a document,
//! so a regression that silently declines everything fails instead of passing
//! vacuously.

#[path = "support/mod.rs"]
mod support;

use pm_expr::{Context, Dialect, EvalError, Expression, JsStr, JsonHost, JsonRef, Value};
use pm_model::json::{Value as Json, parse};
use support::{Mulberry32, encode};

struct Gen(Mulberry32);

impl Gen {
    fn below(&mut self, n: u32) -> u32 {
        self.0.next() % n
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u32) as usize]
    }

    fn number(&mut self) -> String {
        self.pick(&[
            "0", "1", "2", "3", "0.5", "1.5", "10", "100", "-1", "0.1", "1e3", "2.75", "7", "1e-7",
            "360", "0.25",
        ])
        .to_string()
    }

    fn leaf(&mut self) -> String {
        match self.below(10) {
            0..=2 => self.number(),
            3..=5 => self
                .pick(&["time", "t", "value", "idx", "fps", "T", "PI"])
                .to_string(),
            6 => self
                .pick(&[
                    "layer.p.x.v",
                    "layer.p.y.v",
                    "comp.width",
                    "comp.layers.length",
                    "layer.name.length",
                    "value.x",
                    "Math.PI",
                    "Math.E",
                    "Math.SQRT2",
                ])
                .to_string(),
            7 => self
                .pick(&[
                    "param(\"amp\")",
                    "param(\"text\")",
                    "param(\"flag\")",
                    "param(\"missing\")",
                    "param('toString')",
                ])
                .to_string(),
            8 => self.pick(&["true", "false"]).to_string(),
            // Outside the plan: strings, null, unknown names, dynamic keys.
            _ => self
                .pick(&[
                    "\"4\"",
                    "null",
                    "layer.name",
                    "nothing",
                    "comp.layers[0].v",
                    "Math",
                    "clamp",
                ])
                .to_string(),
        }
    }

    fn expr(&mut self, depth: u32) -> String {
        if depth == 0 {
            return self.leaf();
        }
        let d = depth - 1;
        match self.below(12) {
            0 | 1 => self.leaf(),
            2 => {
                let op = self.pick(&["-", "+", "!"]);
                format!("{op}({})", self.expr(d))
            }
            3..=5 => {
                let op = self.pick(&[
                    "+", "-", "*", "/", "%", "**", "<", "<=", ">", ">=", "==", "!=", "===", "!==",
                    "&&", "||",
                ]);
                format!("({} {op} {})", self.expr(d), self.expr(d))
            }
            6 | 7 => format!("({} ? {} : {})", self.expr(d), self.expr(d), self.expr(d)),
            8..=10 => {
                let name = self.pick(&[
                    "clamp", "lerp", "linear", "ease", "random", "wiggle", "bounce", "loop",
                    "pingpong",
                ]);
                let count = self.below(6);
                let args: Vec<String> = (0..count).map(|_| self.expr(d)).collect();
                format!("{name}({})", args.join(", "))
            }
            _ => {
                let name = self.pick(&[
                    "Math.sin",
                    "Math.cos",
                    "Math.abs",
                    "Math.max",
                    "Math.min",
                    "Math.floor",
                    "Math.round",
                    "Math.pow",
                    "Math.atan2",
                    "Math.sqrt",
                    "sin",
                    "max",
                    "Math.sign",
                ]);
                let count = self.below(4);
                let args: Vec<String> = (0..count).map(|_| self.expr(d)).collect();
                format!("{name}({})", args.join(", "))
            }
        }
    }
}

const NUMBERS: [f64; 12] = [
    0.0,
    -0.0,
    0.5,
    1.0,
    2.5,
    -3.0,
    7.25,
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    1e300,
    1.0 / 3.0,
];

fn json_number(n: f64) -> String {
    if n.is_nan() || n.is_infinite() {
        // JSON has no NaN/Infinity; the documents use a string instead.
        "\"x\"".to_string()
    } else if n == 0.0 && n.is_sign_negative() {
        "-0.0".to_string()
    } else {
        format!("{n:?}")
    }
}

fn outcome(result: &Result<Value<JsonRef<'_>>, EvalError>) -> Json {
    match result {
        Ok(v) => encode(v),
        Err(e) => Json::String(e.message().into()),
    }
}

#[test]
fn numeric_plan_equals_the_bytecode_evaluator() {
    // `PM_EXPR_NUMERIC_SEEDS=n` runs n seeds (a longer local soak).
    let seeds: u32 = std::env::var("PM_EXPR_NUMERIC_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    for seed in 0..seeds {
        differential(0x5eed_2026 ^ seed.wrapping_mul(0x9e37_79b9));
    }
}

fn differential(seed: u32) {
    let mut rng = Gen(Mulberry32(seed));
    let mut documents = Vec::new();
    for i in 0..12 {
        let pick = |k: usize| NUMBERS[(i * 5 + k * 7) % NUMBERS.len()];
        let x = if i % 4 == 3 {
            "\"12\"".to_string()
        } else {
            json_number(pick(0))
        };
        let amp = match i % 5 {
            0 => "{\"value\":true}".to_string(),
            1 => "{\"value\":\"3\"}".to_string(),
            2 => "null".to_string(),
            _ => format!("{{\"value\":{}}}", json_number(pick(1))),
        };
        let layer = format!(
            r#"{{"name":"Layer {i}","p":{{"x":{{"v":{x}}},"y":{{"v":{}}}}}}}"#,
            json_number(pick(2))
        );
        let comp = format!(
            r#"{{"width":{},"layers":[{{"v":1}},{{"v":2}}],"params":{{"amp":{amp},"text":{{"value":"abc"}},"flag":{{"value":false}}}}}}"#,
            json_number(pick(3))
        );
        documents.push((parse(&layer).unwrap(), parse(&comp).unwrap()));
    }
    let object_value = parse(r#"{"x":4}"#).unwrap();
    let (mut sources, mut compiled_plans, mut answered, mut scalar_answers) = (0, 0, 0, 0);
    let (mut general_bound, mut general_checked, mut general_revalidated) = (0, 0, 0);
    let mut checked = 0usize;
    for _ in 0..30_000 {
        let depth = 1 + rng.below(4);
        let source = rng.expr(depth);
        let Some(expr) = Expression::compile(&source, Dialect::Legacy) else {
            continue;
        };
        sources += 1;
        let mut has_plan = false;
        for case in 0..6 {
            let (layer, comp) = &documents[rng.below(documents.len() as u32) as usize];
            let number = |rng: &mut Gen| NUMBERS[rng.below(NUMBERS.len() as u32) as usize];
            let value = match case {
                4 => Value::Bool(rng.below(2) == 1),
                5 if rng.below(2) == 0 => JsonHost::value(&object_value),
                5 => Value::String(JsStr::from("5")),
                _ => Value::Number(number(&mut rng)),
            };
            let ctx = Context {
                t: number(&mut rng),
                time: number(&mut rng),
                fps: [30.0, 24.0, 0.0, f64::NAN][rng.below(4) as usize],
                frame: f64::NAN,
                value,
                layer: JsonHost::value(layer),
                comp: JsonHost::value(comp),
                channel: JsStr::from("x"),
                idx: if expr.needs_idx() {
                    f64::from(rng.below(4))
                } else {
                    0.0
                },
            };
            let host = JsonHost::new();
            let tracked = expr.evaluate(&host, &ctx).result;
            let untracked = expr.evaluate_untracked(&host, &ctx);
            assert_eq!(outcome(&tracked), outcome(&untracked), "{source}");
            checked += 1;
            // The general binding: other times and values, and other
            // documents whenever its validation accepts them.
            if let Some((bound, binding)) = expr.bind_general(&host, &ctx) {
                general_bound += 1;
                assert!(
                    expr.general_binding_matches(&host, &ctx, &binding),
                    "{source}"
                );
                for _ in 0..4 {
                    let mut at = ctx.clone();
                    at.t = number(&mut rng);
                    at.time = number(&mut rng);
                    at.value = Value::Number(number(&mut rng));
                    if rng.below(2) == 0 {
                        let (layer, comp) = &documents[rng.below(documents.len() as u32) as usize];
                        at.layer = JsonHost::value(layer);
                        at.comp = JsonHost::value(comp);
                        at.fps = [30.0, 24.0, 0.0, f64::NAN][rng.below(4) as usize];
                    }
                    if !expr.general_binding_matches(&host, &at, &binding) {
                        general_revalidated += 1;
                        continue;
                    }
                    let expected = expr.evaluate(&host, &at).result;
                    assert_eq!(
                        outcome(&expected),
                        outcome(&bound.evaluate_untracked(&host, &at)),
                        "general: {source}"
                    );
                    if let Some(base) = at.value.as_number()
                        && let Some(n) = bound.evaluate_scalars(at.t, at.time, base)
                    {
                        assert_eq!(
                            outcome(&Ok(Value::Number(n))),
                            outcome(&expected),
                            "general scalars: {source}"
                        );
                    }
                    general_checked += 1;
                }
            }
            let Some(bound) = expr.bind_numeric(&host, &ctx) else {
                continue;
            };
            has_plan = true;
            let bits = expr.numeric_fixed_bits(&host, &ctx);
            if let Some(bits) = &bits {
                assert!(expr.numeric_fixed_matches(&host, &ctx, bits), "{source}");
            }
            // The bound plan at other dynamic inputs, against the bytecode.
            for _ in 0..3 {
                let mut at = ctx.clone();
                at.t = number(&mut rng);
                at.time = number(&mut rng);
                if rng.below(3) > 0 {
                    at.value = Value::Number(number(&mut rng));
                }
                let expected = expr.evaluate(&host, &at).result;
                let prepared = bound.evaluate_untracked(&host, &at);
                checked += 1;
                if bits.is_some() {
                    // Only a binding whose fixed inputs are all numbers is
                    // ever reused by the embedding.
                    assert_eq!(outcome(&expected), outcome(&prepared), "bound: {source}");
                    if let Some(base) = at.value.as_number()
                        && let Some(n) = bound.evaluate_scalars(at.t, at.time, base)
                    {
                        scalar_answers += 1;
                        assert_eq!(
                            outcome(&Ok(Value::Number(n))),
                            outcome(&expected),
                            "scalars: {source}"
                        );
                    }
                }
                if let (Ok(Value::Number(a)), Ok(Value::Number(b))) = (&expected, &prepared)
                    && a.to_bits() == b.to_bits()
                {
                    answered += 1;
                }
            }
        }
        if has_plan {
            compiled_plans += 1;
        }
    }
    eprintln!(
        "numeric plan: {sources} sources, {compiled_plans} with plans, {checked} comparisons, \
         {answered} bound numeric answers, {scalar_answers} scalar answers; general: {general_bound} \
         bindings, {general_checked} comparisons, {general_revalidated} rejected by validation"
    );
    assert!(
        general_checked > 10_000,
        "general comparisons: {general_checked}"
    );
    assert!(
        general_revalidated > 100,
        "validation never rejected: {general_revalidated}"
    );
    // The generator mixes in declining constructs; most sources still plan.
    assert!(
        compiled_plans * 3 > sources,
        "plans: {compiled_plans} of {sources}"
    );
    assert!(scalar_answers > 10_000, "scalar answers: {scalar_answers}");
}

#[test]
fn common_channel_expressions_compile_to_scalar_plans() {
    let layer = parse(r#"{"p":{"opacity":{"v":40}}}"#).unwrap();
    let comp = parse(r#"{"layers":[{}],"params":{"amp":{"value":7}}}"#).unwrap();
    let ctx = Context {
        t: 0.5,
        time: 1.5,
        fps: 30.0,
        frame: f64::NAN,
        value: Value::Number(10.0),
        layer: JsonHost::value(&layer),
        comp: JsonHost::value(&comp),
        channel: JsStr::from("x"),
        idx: 3.0,
    };
    for source in [
        "clamp(value + time * 3, 0, 50)",
        "linear(time, 0, 2, 0, 100)",
        "time > 1 ? value * 2 : value + time",
        "ease(time, 0, 3, value, 100) + loop(2, time)",
        "time < 2 ? param(\"amp\") : value",
        "value + (idx % 2 ? time : -time)",
        "lerp(value, 100, time / 10)",
        "pingpong(time, 3) * value",
        "wiggle(2, 10) + value",
        "bounce(t) * 100 + random(idx)",
        "!(time > 1) * 5 + (value >= 10 && value || 3)",
        "Math.PI * 2 * time + layer.p.opacity.v",
        "time === 1.5 ? 1 : 0",
    ] {
        let expr = Expression::compile(source, Dialect::Legacy).unwrap();
        let host = JsonHost::new();
        let bound = expr
            .bind_numeric(&host, &ctx)
            .unwrap_or_else(|| panic!("no plan: {source}"));
        let n = bound
            .evaluate_scalars(ctx.t, ctx.time, 10.0)
            .unwrap_or_else(|| panic!("no scalar plan: {source}"));
        let expected = expr.evaluate(&host, &ctx).result.unwrap();
        assert_eq!(
            expected.as_number().map(f64::to_bits),
            Some(n.to_bits()),
            "{source}"
        );
    }
    // A boolean result, or `===` across types, is left to the bytecode.
    for source in [
        "time > 1",
        "!value",
        "(time > 1) === 1",
        "time > 1 || value",
    ] {
        let expr = Expression::compile(source, Dialect::Legacy).unwrap();
        assert!(
            expr.bind_numeric(&JsonHost::new(), &ctx).is_none(),
            "{source}"
        );
    }
}
