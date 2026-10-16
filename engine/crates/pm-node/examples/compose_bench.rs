//! The viewer's document frame cost (docs/engine/tracer.md, "Measurements"):
//! composes a canonical document (a file, e.g. a synthetic project's
//! `PM.EngineDoc.snapshot().json`) at consecutive frame times into a
//! viewer-sized working texture and reports the CPU time of each compose
//! (evaluation, planning, encoding) and the wall time including the GPU.
//!
//!   cargo run --release -p pm-node --example compose_bench -- <document.json> <composition> [frames] [width] [height]

#[cfg(target_os = "macos")]
fn main() {
    use pm_gpu::bridge_out::display::WorkingTexture;
    use pm_node::present::compose::Composer;
    use pm_node::present::document::MediaMap;
    use pm_protocol::color::ViewerColorSpace;
    use pm_protocol::common::{Seq, Size, TICKS_PER_SECOND, Ticks};
    use pm_protocol::frame::{MediaPreference, PreviewResolution, ViewerState};
    use std::time::Instant;

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: compose_bench <document.json> <composition> [frames] [width] [height]");
        std::process::exit(2);
    }
    let text = std::fs::read_to_string(&args[1]).expect("read the document");
    let composition = args[2].clone();
    let frames: u32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(120);
    let width: u32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(1496);
    let height: u32 = args.get(5).and_then(|v| v.parse().ok()).unwrap_or(904);
    let parse = Instant::now();
    let doc = pm_model::Document::new(
        Some(pm_model::json::parse(&text).expect("parse the document")),
        1,
    );
    eprintln!("parsed in {:.1} ms", parse.elapsed().as_secs_f64() * 1e3);
    let comp = pm_eval::Comp::resolve(&doc, &composition).expect("the composition");
    let number = |key: &str| pm_eval::node::to_number(comp.member(key));
    let (cw, ch) = (number("w"), number("h"));
    let fps = number("fps");
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 30.0 };
    // Fit the composition in the view at DPR 2, as the viewer does.
    let zoom = (f64::from(width) / 2.0 / cw).min(f64::from(height) / 2.0 / ch);
    let state = ViewerState {
        composition: composition.as_str().into(),
        size: Size::new(width, height),
        device_pixel_ratio: 2.0,
        zoom,
        pan: [0.0, 0.0],
        resolution: PreviewResolution::Automatic,
        media: MediaPreference::Automatic,
        color_space: ViewerColorSpace::DisplayP3,
        view: None,
        play_every_frame: false,
        edr: false,
        view_seq: Seq::new(1).unwrap(),
        vsync: None,
    };
    let gpu = pm_gpu::Gpu::new("compose bench").expect("a GPU");
    let working = WorkingTexture::new(&gpu.device, Size::new(width, height));
    let mut composer = Composer::new(&gpu.device, &gpu.queue);
    let media = MediaMap::default();
    let mut cpu = Vec::new();
    let mut wall = Vec::new();
    for k in 0..frames {
        let time = Ticks::new((f64::from(k) / fps * TICKS_PER_SECOND as f64).round() as i64)
            .unwrap();
        let start = Instant::now();
        let composed = composer
            .compose(&doc, &composition, &media, time, &state, &working)
            .expect("compose");
        let index = gpu.queue.submit([]);
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(std::time::Duration::from_secs(5)),
            })
            .expect("poll");
        if k == 0 && !composed.failures.is_empty() {
            eprintln!("{} layer failures", composed.failures.len());
        }
        cpu.push(composed.cpu_us as f64 / 1e3);
        wall.push(start.elapsed().as_secs_f64() * 1e3);
    }
    let summary = |xs: &mut Vec<f64>| {
        // The first frame builds pipelines and caches.
        let first = xs.remove(0);
        xs.sort_by(|a, b| a.total_cmp(b));
        let at = |p: f64| xs[((xs.len() - 1) as f64 * p) as usize];
        format!(
            "first {first:.2} p50 {:.2} p95 {:.2} max {:.2} ms",
            at(0.5),
            at(0.95),
            at(1.0)
        )
    };
    println!("{frames} frames of {composition} ({cw}x{ch} at {zoom:.3}x in {width}x{height} px)");
    println!("compose CPU: {}", summary(&mut cpu));
    println!("compose + GPU: {}", summary(&mut wall));
}

#[cfg(not(target_os = "macos"))]
fn main() {}
