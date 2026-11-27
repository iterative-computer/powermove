//! Native timings for the evaluator, on a synthetic composition shaped like
//! the S5 project (884 layers, 9 transform channels each, a few percent
//! animated).
//!
//!   cargo run --release -p pm-eval --example bench_eval
//!
//! Prints one line per measurement. Timings are medians of 9 trials.

use std::hint::black_box;
use std::time::Instant;

use pm_eval::{Comp, Evaluator, PropAddress, PropId, ValueKind};
use pm_model::json::parse;
use pm_model::{Document, JsString, Patch, PathPart, Value};

const CHANNELS: [&str; 9] = [
    "position.x",
    "position.y",
    "anchor.x",
    "anchor.y",
    "scale.x",
    "scale.y",
    "rotation",
    "skew",
    "opacity",
];

fn project(layers: usize) -> String {
    let mut out = String::from(
        r#"{"id":"P","compId":"C","comps":{"C":{"id":"C","fps":30,"dur":60,"layers":["#,
    );
    let mut seed = 7u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for i in 0..layers {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            r#"{{"id":"L{i}","type":"shape","from":0,"dur":60,"on":true,"p":{{"#
        ));
        for (c, ch) in CHANNELS.iter().enumerate() {
            if c > 0 {
                out.push(',');
            }
            let animated = next() < 0.05;
            let kf = if animated {
                let keys: Vec<String> = (0..4)
                    .map(|k| {
                        format!(
                            r#"{{"t":{},"v":{},"inInterp":"bezier","outInterp":"bezier","inEase":{{"speed":{},"influence":{}}},"outEase":{{"speed":{},"influence":{}}}}}"#,
                            k as f64 * 2.5,
                            (next() * 500.0).round(),
                            (next() * 200.0 - 100.0).round(),
                            (next() * 90.0 + 5.0).round(),
                            (next() * 200.0 - 100.0).round(),
                            (next() * 90.0 + 5.0).round()
                        )
                    })
                    .collect();
                keys.join(",")
            } else {
                String::new()
            };
            out.push_str(&format!(
                r#""{ch}":{{"v":{},"kf":[{kf}],"expr":null}}"#,
                (next() * 100.0).round()
            ));
        }
        out.push_str("}}");
    }
    out.push_str("]}}}");
    out
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn time_per(iterations: usize, mut f: impl FnMut(usize)) -> f64 {
    let trials: Vec<f64> = (0..9)
        .map(|_| {
            let start = Instant::now();
            for i in 0..iterations {
                f(i);
            }
            start.elapsed().as_nanos() as f64 / iterations as f64
        })
        .collect();
    median(trials)
}

fn main() {
    let text = project(884);
    let start = Instant::now();
    let mut doc = Document::new(Some(parse(&text).unwrap()), 1);
    println!(
        "load (parse + tree), 884 layers, {} KB: {:.2} ms",
        text.len() / 1024,
        start.elapsed().as_secs_f64() * 1e3
    );

    let mut ev = Evaluator::new();
    let mut ids: Vec<PropId> = Vec::new();
    let mut animated: Vec<PropId> = Vec::new();
    let cx = Comp::resolve(&doc, "C").unwrap();
    for (i, layer) in cx.layers().iter().enumerate() {
        for ch in CHANNELS {
            let id = ev.intern(PropAddress::channel("C", &format!("L{i}"), ch));
            ids.push(id);
            let kf = layer
                .get("p")
                .and_then(|p| p.get(ch))
                .and_then(|p| p.get("kf"));
            if matches!(kf.map(|k| k.kind()), Some(pm_model::NodeKind::Array(a)) if !a.is_empty()) {
                animated.push(id);
            }
        }
    }
    println!("channels: {} ({} animated)", ids.len(), animated.len());

    let mut out = vec![0.0; ids.len()];
    let mut kinds = vec![ValueKind::Number; ids.len()];
    let start = Instant::now();
    ev.ev_batch(&doc, &ids, 1.0, &mut out, &mut kinds);
    println!(
        "first batch (plan every id): {:.3} ms",
        start.elapsed().as_secs_f64() * 1e3
    );

    let n = ids.len();
    let cached = time_per(2_000_000, |i| {
        black_box(ev.ev_id(&doc, ids[i % n], 1.0).ok());
    });
    println!("ev_id cached, all channels: {cached:.1} ns");
    let an = animated.len();
    let cached_anim = time_per(2_000_000, |i| {
        black_box(ev.ev_id(&doc, animated[i % an], 1.0).ok());
    });
    println!("ev_id cached, animated: {cached_anim:.1} ns");
    let mut t = 1.0;
    let new_time = time_per(200_000, |i| {
        if i % an == 0 {
            t += 1.0 / 30.0;
        }
        black_box(ev.ev_id(&doc, animated[i % an], t).ok());
    });
    println!("ev_id at a new time, animated (curve solve): {new_time:.1} ns");
    let batch = time_per(200, |_| {
        ev.ev_batch(&doc, &ids, 1.0, &mut out, &mut kinds);
    }) / n as f64;
    println!("ev_batch per value, cached: {batch:.2} ns");

    // A property edit: one layer's position.x, then a whole batch at the new revision.
    let mut revision_ms = Vec::new();
    for round in 0..50 {
        let path = vec![
            PathPart::from("comps"),
            PathPart::from("C"),
            PathPart::from("layers"),
            PathPart::Index((round * 17 % 884) as f64),
            PathPart::from("p"),
            PathPart::from("position.x"),
            PathPart::from("v"),
        ];
        doc.apply(&[Patch::set(path, Value::Number(round as f64))])
            .unwrap();
        let start = Instant::now();
        ev.ev_batch(&doc, &ids, 1.0, &mut out, &mut kinds);
        revision_ms.push(start.elapsed().as_secs_f64() * 1e3);
    }
    let stats = ev.cache_stats();
    println!(
        "batch after a property edit (revalidate {} ids): median {:.3} ms; stats {stats:?}",
        ids.len(),
        median(revision_ms)
    );

    // The node API, uncached: prepare and evaluate per call.
    let cx = Comp::resolve(&doc, "C").unwrap();
    let layers = cx.layers();
    let key = JsString::from("position.x");
    let animated_layer = layers
        .iter()
        .find(|l| {
            matches!(l.get("p").and_then(|p| p.get("position.x")).and_then(|p| p.get("kf")).map(|k| k.kind()),
                Some(pm_model::NodeKind::Array(a)) if !a.is_empty())
        })
        .unwrap();
    let mut ev2 = Evaluator::new();
    let uncached = time_per(200_000, |i| {
        black_box(
            ev2.ev(&cx, Some(animated_layer), &key, 1.0 + i as f64 * 1e-6)
                .ok(),
        );
    });
    println!(
        "ev by node (uncached: adopt, prepare, evaluate), 4-key bezier track: {uncached:.1} ns"
    );
}
