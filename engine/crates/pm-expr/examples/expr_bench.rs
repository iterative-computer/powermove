//! Rough evaluation cost: `cargo run -p pm-expr --release --example expr_bench`.

use std::hint::black_box;
use std::time::Instant;

use pm_expr::{Context, Dialect, Expression, JsStr, JsonHost, Value};

fn main() {
    let layer = pm_model::json::parse(
        r#"{"name":"Title","from":0.5,"p":{"x":{"v":4,"kf":[{"t":0,"v":1}]}},"tags":["a","b"]}"#,
    )
    .unwrap();
    let comp =
        pm_model::json::parse(r#"{"w":1920,"layers":[{},{},{}],"params":{"gain":{"value":3}}}"#)
            .unwrap();
    let sources = [
        "time * 90",
        "value + Math.sin(t) * 10",
        "320 + wiggle(2, 120)",
        "60 + random(7) * 500",
        "linear(time, 0, 2.5, 60, 580) * 0.5 + ease(time, 0, 2.5, 60, 580) * 0.5",
        "60 + param(\"gain\") * Math.abs(Math.sin(time * PI))",
        "layer.p.x.v * comp.w / 100 + layer.tags.length",
        "idx === 2 ? layer.name + \" \" + comp.w : \"no\"",
    ];
    let host = JsonHost::new();
    for source in sources {
        let compile_start = Instant::now();
        let n_compile = 20_000;
        for _ in 0..n_compile {
            black_box(Expression::compile(black_box(source), Dialect::Native));
        }
        let compile_ns = compile_start.elapsed().as_nanos() as f64 / n_compile as f64;
        let expr = Expression::compile(source, Dialect::Native).unwrap();
        let n = 200_000;
        let mut ctx = Context {
            t: 0.0,
            time: 0.5,
            fps: 60.0,
            frame: 0.0,
            value: Value::Number(10.0),
            layer: JsonHost::value(&layer),
            comp: JsonHost::value(&comp),
            channel: JsStr::from("position.x"),
            idx: 2.0,
        };
        let start = Instant::now();
        for i in 0..n {
            ctx.t = i as f64 / 60.0;
            ctx.time = ctx.t + 0.5;
            ctx.frame = i as f64;
            black_box(expr.evaluate(&host, black_box(&ctx)));
        }
        let eval_ns = start.elapsed().as_nanos() as f64 / n as f64;
        println!("{eval_ns:8.0} ns/eval {compile_ns:8.0} ns/compile  {source}");
    }
}
