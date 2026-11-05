//! Synthetic headless CPU evidence; never a claim about macOS scanout.
#[path = "../tests/support/render_fixture.rs"]
#[allow(dead_code)]
mod fixture;
use pm_design::{Accessibility, Scheme, Tokens};
use std::{hint::black_box, time::Instant};
fn percentile(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}
fn main() {
    let t = Tokens::new(Scheme::Dark, Accessibility::default());
    let start = Instant::now();
    let scene = fixture::large_scene(60_000, &t);
    let build = start.elapsed();
    let mut v = fixture::viewport(&t);
    v.height = 1080.;
    let mut times = vec![];
    let mut wire = vec![];
    let mut overlays = vec![];
    let mut max_ops = 0;
    let mut max_bytes = 0;
    let mut max_rows = 0;
    for i in 0..1200 {
        v.scroll_y = (i * 47 % 59_900) as f64 * t.metrics.track_row as f64;
        let start = Instant::now();
        let f = scene.prepare(v, &t).unwrap();
        let dt = start.elapsed().as_secs_f64() * 1e6;
        let start = Instant::now();
        let bytes = black_box(f.wire().to_string()).len();
        let ser = start.elapsed().as_secs_f64() * 1e6;
        let start = Instant::now();
        black_box(f.playhead(1. + (i % 300) as f64 / 120., &t).unwrap());
        let overlay = start.elapsed().as_secs_f64() * 1e6;
        if i >= 120 {
            times.push(dt);
            wire.push(ser);
            overlays.push(overlay);
        }
        max_ops = max_ops.max(f.ops.len());
        max_bytes = max_bytes.max(bytes);
        max_rows = max_rows.max(f.range.len());
    }
    let p95 = percentile(&mut times, 0.95);
    let p99 = percentile(&mut times, 0.99);
    let ser = percentile(&mut wire, 0.95);
    let overlay = percentile(&mut overlays, 0.95);
    println!(
        "{}",
        serde_json::json!({"layers":60_000,"samples":1080,"viewport":[v.width,v.height],"build_ms":build.as_secs_f64()*1000.,"max_rows":max_rows,"max_ops":max_ops,"max_wire_bytes":max_bytes,"prepare_p95_us":p95,"prepare_p99_us":p99,"wire_p95_us":ser,"playhead_p95_us":overlay,"budget_us":1_000_000./120.,"cpu_headroom_us":1_000_000./240.})
    );
    assert!(
        p95 < 1_000_000. / 240.,
        "headless preparation exceeds the half-frame CPU budget"
    );
}
