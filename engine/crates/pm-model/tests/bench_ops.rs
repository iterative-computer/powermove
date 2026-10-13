//! Timings for the S5 structural-edit budget on an 884-layer composition:
//! decode a canonical binary body and apply it to the revisioned document.
//! Not a gate (timings depend on the machine and its load); run with
//!
//!   cargo test --release -p pm-model --test bench_ops -- --ignored --nocapture

use std::time::Instant;

use pm_model::Document;
use pm_model::binary::{decode, encode};
use pm_model::json::{Value, parse};
use pm_model::ops::diff_ops;
use pm_model::patch::{Patch, diff_patches};

fn layer(i: usize) -> String {
    let channels: Vec<String> = [
        "position.x",
        "position.y",
        "scale.x",
        "scale.y",
        "rotation",
        "opacity",
        "anchor.x",
        "anchor.y",
    ]
    .iter()
    .map(|k| format!(r#""{k}":{{"v":{i},"kf":[],"expr":null}}"#))
    .collect();
    let path: Vec<String> = (0..40)
        .map(|k| format!("[{k},{},{}]", k * 2, k * 3))
        .collect();
    format!(
        r##"{{"id":"layer-{i}","name":"Layer {i}","type":"{}","from":{},"dur":5,"p":{{{}}},"d":{{"fill":"#336699","stroke":null,"path":[{}]}}}}"##,
        if i.is_multiple_of(7) { "text" } else { "shape" },
        i as f64 / 30.0,
        channels.join(","),
        path.join(",")
    )
}

fn project(layers: &[String]) -> Value {
    parse(&format!(
        r#"{{"compId":"c1","comps":{{"c1":{{"id":"c1","layers":[{}]}}}}}}"#,
        layers.join(",")
    ))
    .unwrap()
}

fn percentile(mut xs: Vec<f64>, q: f64) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[((xs.len() as f64 * q) as usize).min(xs.len() - 1)]
}

#[test]
#[ignore = "timing report, not a gate"]
fn structural_edits_on_884_layers() {
    let layers: Vec<String> = (0..884).map(layer).collect();
    let before = project(&layers);
    let mut reorder = layers.clone();
    let moved = reorder.remove(500);
    reorder.insert(0, moved);
    let mut insert = layers.clone();
    insert.insert(10, layer(0).replace("layer-0", "new-layer"));
    let mut delete = layers.clone();
    delete.remove(300);
    println!("whole layers JSON: {} bytes", layers.join(",").len() + 2);
    for (kind, after) in [("reorder", reorder), ("insert", insert), ("delete", delete)] {
        let after = project(&after);
        let body = encode(&diff_ops(Some(&before), Some(&after))).unwrap();
        let (leaf, _) = diff_patches(Some(&before), Some(&after));
        let leaf_body = encode(&leaf.iter().cloned().map(Into::into).collect::<Vec<_>>()).unwrap();
        let (mut ops_ms, mut leaf_ms) = (Vec::new(), Vec::new());
        for round in 0..60u64 {
            let mut doc = Document::new(Some(before.clone()), 1);
            let t = Instant::now();
            doc.apply_ops_at(&decode(&body).unwrap(), 2 + round)
                .unwrap();
            ops_ms.push(t.elapsed().as_secs_f64() * 1e3);
            let mut doc = Document::new(Some(before.clone()), 1);
            let t = Instant::now();
            doc.apply_ops_at(&decode(&leaf_body).unwrap(), 2 + round)
                .unwrap();
            leaf_ms.push(t.elapsed().as_secs_f64() * 1e3);
        }
        println!(
            "{kind}: id-keyed body {} bytes, decode+apply p50 {:.3} p95 {:.3} ms | whole-array leaf body {} bytes, decode+apply p50 {:.3} p95 {:.3} ms",
            body.len(),
            percentile(ops_ms.clone(), 0.5),
            percentile(ops_ms, 0.95),
            leaf_body.len(),
            percentile(leaf_ms.clone(), 0.5),
            percentile(leaf_ms, 0.95)
        );
    }
    let property = vec![
        Patch::set(
            pm_model::path!["comps", "c1", "layers", 42, "p", "position.x", "v"],
            Value::Number(7.0),
        )
        .into(),
    ];
    let body = encode(&property).unwrap();
    let mut doc = Document::new(Some(before), 1);
    let mut times = Vec::new();
    for round in 0..5_000u64 {
        let t = Instant::now();
        doc.apply_ops_at(&decode(&body).unwrap(), 2 + round)
            .unwrap();
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!(
        "property edit: {} bytes, decode+apply p50 {:.4} p95 {:.4} ms",
        body.len(),
        percentile(times.clone(), 0.5),
        percentile(times, 0.95)
    );
}
