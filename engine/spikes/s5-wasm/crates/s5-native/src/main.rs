//! Native half of spike S5.
//!
//! s5-native vectors <project.json> <times.json> <out.bin>
//!     Evaluate every transform channel of every layer and every world matrix
//!     at each time, in the same order as `js/bench.mjs`, as raw f64 bytes.
//! s5-native compare <a.bin> <b.bin>
//!     Bitwise comparison of two vector files.
//! s5-native bench <project.json>
//!     Native reference timings for the same operations as the wasm runs.

use s5_core::Mirror;
use s5_core::patch::encode_frames;
use serde_json::Value;
use std::time::Instant;

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Intern every transform channel in document order (layer order, then the
/// channel keys in JSON order), matching `internAll` in `js/evaluator.mjs`.
fn intern_all(m: &mut Mirror, project: &Value) -> Vec<u32> {
    let mut ids = Vec::new();
    for layer in project["layers"].as_array().expect("layers") {
        let id = layer["id"].as_str().expect("layer id");
        if let Some(p) = layer["p"].as_object() {
            for key in p.keys() {
                ids.push(m.intern(id, key));
            }
        }
    }
    ids
}

fn vectors(project_path: &str, times_path: &str, out_path: &str) {
    let bytes = read(project_path);
    let project: Value = serde_json::from_slice(&bytes).expect("project JSON");
    let times: Vec<f64> = serde_json::from_slice(&read(times_path)).expect("times JSON");
    let mut m = Mirror::new();
    m.load(&bytes).expect("load");
    let ids = intern_all(&mut m, &project);
    let per = ids.len() + m.layer_count() * 6;
    let mut out = vec![0.0f64; per * times.len()];
    for (k, &t) in times.iter().enumerate() {
        let row = &mut out[k * per..(k + 1) * per];
        let (values, matrices) = row.split_at_mut(ids.len());
        m.ev_batch(&ids, t, values);
        m.world_all(t, matrices);
    }
    let raw: Vec<u8> = out.iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(out_path, raw).expect("write vectors");
    println!(
        "{} values, {} handles, {} layers, {} times",
        out.len(),
        ids.len(),
        m.layer_count(),
        times.len()
    );
}

fn compare(a: &str, b: &str) {
    let a = read(a);
    let b = read(b);
    if a.len() != b.len() {
        println!("LENGTH MISMATCH {} vs {}", a.len(), b.len());
        std::process::exit(1);
    }
    let mut differing = 0usize;
    let mut first = Vec::new();
    for (i, (x, y)) in a.chunks_exact(8).zip(b.chunks_exact(8)).enumerate() {
        if x != y {
            differing += 1;
            if first.len() < 5 {
                let fx = f64::from_le_bytes(x.try_into().unwrap());
                let fy = f64::from_le_bytes(y.try_into().unwrap());
                first.push(format!("#{i}: {fx:e} vs {fy:e}"));
            }
        }
    }
    println!(
        "{} values compared, {} differ{}",
        a.len() / 8,
        differing,
        if first.is_empty() {
            String::new()
        } else {
            format!(": {}", first.join(", "))
        }
    );
    if differing > 0 {
        std::process::exit(1);
    }
}

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[xs.len() / 2]
}

fn bench(project_path: &str) {
    let bytes = read(project_path);
    let project: Value = serde_json::from_slice(&bytes).expect("project JSON");
    let mut m = Mirror::new();
    let t0 = Instant::now();
    m.load(&bytes).expect("load");
    println!("load: {:.3} ms", t0.elapsed().as_secs_f64() * 1e3);
    let ids = intern_all(&mut m, &project);
    for &h in &ids {
        m.ev(h, 10.0);
    }
    let rounds = 200;
    let mut sink = 0.0;
    let per_call = |m: &mut Mirror, sink: &mut f64, t: &dyn Fn(usize) -> f64| {
        let mut trials = Vec::new();
        for _ in 0..9 {
            let s = Instant::now();
            let mut k = 0usize;
            for _ in 0..rounds {
                for &h in &ids {
                    *sink += m.ev(h, t(k));
                    k += 1;
                }
            }
            trials.push(s.elapsed().as_secs_f64() * 1e9 / (rounds * ids.len()) as f64);
        }
        median(trials)
    };
    println!(
        "ev cached: {:.2} ns",
        per_call(&mut m, &mut sink, &|_| 10.0)
    );
    println!(
        "ev new time each call: {:.2} ns",
        per_call(&mut m, &mut sink, &|k| 20.0 + (k % 100_000) as f64 * 1e-4)
    );
    let mut out = vec![0.0; m.layer_count() * 6];
    let mut trials = Vec::new();
    for r in 0..50 {
        let s = Instant::now();
        m.world_all(30.0 + r as f64 * 1e-3, &mut out);
        trials.push(s.elapsed().as_secs_f64() * 1e6);
    }
    println!(
        "world_all ({} layers): {:.1} us",
        m.layer_count(),
        median(trials)
    );
    // Property edit
    let mut trials = Vec::new();
    for k in 0..2000 {
        let frames = encode_frames(&[
            (
                1,
                format!("[\"layers\",{},\"p\",\"position.x\",\"v\"]", 100 + k % 700),
                Some(format!("{}", k as f64 * 0.5)),
            ),
            (1, "[\"revision\"]".into(), Some(k.to_string())),
        ]);
        let s = Instant::now();
        m.apply_framed(&frames).expect("apply");
        trials.push(s.elapsed().as_secs_f64() * 1e3);
    }
    println!("property edit: {:.4} ms", median(trials));
    // Structural reorder: full array
    let mut layers: Vec<Value> = serde_json::from_slice(&m.to_json())
        .map(|v: Value| v["layers"].as_array().unwrap().clone())
        .unwrap();
    let mut trials = Vec::new();
    for k in 0..50 {
        let moved = layers.remove(k * 7 % layers.len());
        layers.insert((k * 13) % layers.len(), moved);
        let json = serde_json::to_string(&layers).unwrap();
        let frames = encode_frames(&[(1, "[\"layers\"]".into(), Some(json))]);
        let s = Instant::now();
        m.apply_framed(&frames).expect("apply");
        trials.push(s.elapsed().as_secs_f64() * 1e3);
        assert_eq!(m.stats().layers_parsed, 0);
    }
    println!(
        "structural reorder (array patch, apply only): {:.3} ms",
        median(trials)
    );
    println!("sink {}", sink.is_finite());
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("vectors") if args.len() == 5 => vectors(&args[2], &args[3], &args[4]),
        Some("compare") if args.len() == 4 => compare(&args[2], &args[3]),
        Some("bench") if args.len() == 3 => bench(&args[2]),
        _ => {
            eprintln!(
                "usage: s5-native vectors <project.json> <times.json> <out.bin> | compare <a> <b> | bench <project.json>"
            );
            std::process::exit(2);
        }
    }
}
