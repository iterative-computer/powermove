//! Print the spacing of CVDisplayLink vsync timestamps on the main display: how far the
//! reported host times stray from the nominal refresh period.
//!
//! Usage: `cargo run --release --example vsync_probe [seconds]`

use s8_contention::{display::DisplayLink, stats};

fn main() {
    let seconds: f64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let dl = DisplayLink::start().expect("display link");
    let nominal = dl.refresh_ns;
    while dl.rx.try_recv().is_ok() {}
    let mut now = Vec::new();
    let mut out = Vec::new();
    let mut arrival = Vec::new();
    let end = s8_contention::clock::now_ns() + (seconds * 1e9) as u64;
    while s8_contention::clock::now_ns() < end {
        let v = dl.rx.recv().unwrap();
        arrival.push(s8_contention::clock::now_ns() as f64 - v.now_ns as f64);
        now.push(v.now_ns);
        out.push(v.output_ns);
    }
    let gaps = |t: &[u64]| -> Vec<f64> { t.windows(2).map(|w| (w[1] - w[0]) as f64 / 1e6).collect() };
    let g = gaps(&now);
    let span = (now[now.len() - 1] - now[0]) as f64;
    println!(
        "nominal {:.4} ms, callbacks {}, span/callbacks {:.4} ms",
        nominal / 1e6,
        now.len(),
        span / (now.len() - 1) as f64 / 1e6
    );
    println!("now gaps ms {:?}", stats::dist(&g).rounded(3));
    println!("output gaps ms {:?}", stats::dist(&gaps(&out)).rounded(3));
    println!(
        "callback arrival after now ms {:?}",
        stats::dist(&arrival.iter().map(|a| a / 1e6).collect::<Vec<_>>()).rounded(3)
    );
    println!("measure_refresh(120): {:.4} ms", dl.measure_refresh(120) / 1e6);
    let off: usize = g.iter().filter(|x| (*x - nominal / 1e6).abs() > 0.5).count();
    println!("gaps off nominal by > 0.5 ms: {off} of {}", g.len());
}
