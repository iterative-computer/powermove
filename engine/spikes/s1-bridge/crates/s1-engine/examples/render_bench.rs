//! Per-frame render cost into an IOSurface pool surface, outside Electron.
//! Usage: cargo run --release -p s1-engine --example render_bench [bgra|rgbaf16] [extra_passes] [frames]
use s1_engine::pool::Surface;
use s1_engine::render::{FrameParams, Gpu, SceneKind};
use s1_mach::wire::PixelFormat;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let format = match args.next().as_deref() {
        Some("rgbaf16") => PixelFormat::Rgba16F,
        _ => PixelFormat::Bgra8,
    };
    let extra: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let frames: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(240);
    let gpu = Gpu::new(3840, 2160, format, extra).unwrap();
    let surfaces: Vec<Surface> = (0..4).map(|i| Surface::create(&gpu.device, i, 3840, 2160, format).unwrap()).collect();
    for scene in ["pattern", "cubes"] {
        let mut times = Vec::new();
        for seq in 0..frames {
            let p = FrameParams {
                time_s: seq as f32 / 60.0,
                seq,
                revision: seq,
                marker: [100.0, 100.0, 96.0, 96.0],
                scene: if scene == "pattern" { SceneKind::Pattern } else { SceneKind::Cubes { yaw: seq as f32 * 0.01, pitch: 0.3, distance: 26.0, walk: 0.0 } },
            };
            let t = Instant::now();
            let idx = gpu.render(&surfaces[(seq % 4) as usize], &p);
            gpu.wait(idx).unwrap();
            times.push(t.elapsed().as_secs_f64() * 1e3);
        }
        times.sort_by(|a, b| a.total_cmp(b));
        let pct = |p: f64| times[((times.len() as f64 - 1.0) * p) as usize];
        println!("{format:?} extra={extra} {scene}: p50 {:.2} ms p95 {:.2} ms max {:.2} ms", pct(0.5), pct(0.95), pct(1.0));
    }
}
