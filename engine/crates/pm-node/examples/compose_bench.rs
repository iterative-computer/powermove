//! The viewer's document frame cost (docs/engine/tracer.md, "Measurements"):
//! composes a canonical document (a file, e.g. a synthetic project's
//! `PM.EngineDoc.snapshot().json`) at consecutive frame times into a
//! viewer-sized working texture and reports the CPU time of each compose
//! (evaluation, planning, encoding) and the wall time including the GPU.
//!
//!   cargo run --release -p pm-node --example compose_bench -- <document.json> <composition> [frames] [width] [height]
//!
//! `PM_BENCH_EDIT=drag` makes every frame a new revision that moves one
//! layer (a layer drag: one leaf edit per frame at a fixed time);
//! `PM_BENCH_LAYER=<n>` edits the n-th solid layer every frame (one layer
//! dragged; layers are listed top first). `PM_BENCH_REUSE=0` plans every layer of every frame (no draw reuse).
//! `PM_BENCH_EDIT=playback` edits one layer's opacity per frame while the
//! time advances (inspector edits during playback, at the worst rate).
//! docs/engine/perf.md records the runs.

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
    let mut doc = pm_model::Document::new(
        Some(pm_model::json::parse(&text).expect("parse the document")),
        1,
    );
    eprintln!("parsed in {:.1} ms", parse.elapsed().as_secs_f64() * 1e3);
    let edit = std::env::var("PM_BENCH_EDIT").ok();
    let (cw, ch, fps, solids) = {
        let comp = pm_eval::Comp::resolve(&doc, &composition).expect("the composition");
        let number = |key: &str| pm_eval::node::to_number(comp.member(key));
        // Indexes of the solid layers (the layers an edit touches).
        let solids: Vec<usize> = comp
            .layers()
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                pm_eval::node::string(pm_eval::node::member(Some(l), "type"))
                    .is_some_and(|t| t.to_string_lossy() == "solid")
            })
            .map(|(i, _)| i)
            .collect();
        (number("w"), number("h"), number("fps"), solids)
    };
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    // Fit the composition in the view at DPR 2, as the viewer does.
    let zoom = (f64::from(width) / 2.0 / cw).min(f64::from(height) / 2.0 / ch);
    let state = ViewerState {
        headless: None,
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
        depth_id: None,
        canvas: None,
    };
    let gpu = pm_gpu::Gpu::new("compose bench").expect("a GPU");
    let working = WorkingTexture::new(&gpu.device, Size::new(width, height));
    let mut composer = Composer::new(&gpu.device, &gpu.queue);
    // PM_BENCH_REUSE=0: every frame plans every layer (no draw reuse).
    if std::env::var("PM_BENCH_REUSE").is_ok_and(|v| v == "0") {
        composer.set_draw_reuse(false);
    }
    let media = MediaMap::default();
    let mut cpu = Vec::new();
    let mut wall = Vec::new();
    let mut apply_ms = Vec::new();
    let fixed: Option<usize> = std::env::var("PM_BENCH_LAYER")
        .ok()
        .and_then(|v| v.parse().ok());
    for k in 0..frames {
        let at = match edit.as_deref() {
            Some("drag") => 0,
            _ => k,
        };
        let time =
            Ticks::new((f64::from(at) / fps * TICKS_PER_SECOND as f64).round() as i64).unwrap();
        if let (Some(mode), false) = (edit.as_deref(), solids.is_empty()) {
            use pm_model::json::Value;
            use pm_model::patch::{Patch, PathPart};
            // PM_BENCH_LAYER=<n>: every frame edits the n-th solid (a
            // drag of one layer); else a different solid each frame.
            let layer = match fixed {
                Some(n) => solids[n.min(solids.len() - 1)],
                None => solids[(k as usize * 37) % solids.len()],
            };
            let (key, value) = match mode {
                "drag" => ("position.x", 100.0 + f64::from(k)),
                _ => ("opacity", 30.0 + f64::from(k % 60)),
            };
            let path = vec![
                PathPart::Key("comps".into()),
                PathPart::Key(composition.as_str().into()),
                PathPart::Key("layers".into()),
                PathPart::Index(layer as f64),
                PathPart::Key("p".into()),
                PathPart::Key(key.into()),
                PathPart::Key("v".into()),
            ];
            let start = Instant::now();
            doc.apply(&[Patch::set(path, Value::Number(value))])
                .expect("apply the edit");
            apply_ms.push(start.elapsed().as_secs_f64() * 1e3);
        }
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
    // What each new revision costs the presenter besides drawing: the
    // routing decision (capability check over the document) and a copy of
    // the document (the audio thread mixes from its own).
    let mut route_ms = Vec::new();
    let mut clone_ms = Vec::new();
    let mut apply_summary = String::new();
    if !apply_ms.is_empty() {
        apply_summary = format!("edit apply: {}", summary(&mut apply_ms));
    }
    // PM_BENCH_ROUTES: how many routing decisions to time (a profile of
    // the capability check wants many).
    let routes: usize = std::env::var("PM_BENCH_ROUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    for _ in 0..routes.max(2) {
        let start = Instant::now();
        let reasons = pm_render::unsupported_with_media(&doc, &composition, &media);
        route_ms.push(start.elapsed().as_secs_f64() * 1e3);
        assert!(reasons.is_empty(), "{reasons:?}");
        let start = Instant::now();
        let copy = doc.clone();
        clone_ms.push(start.elapsed().as_secs_f64() * 1e3);
        drop(copy);
    }
    println!("{frames} frames of {composition} ({cw}x{ch} at {zoom:.3}x in {width}x{height} px)");
    println!("routing (capability check): {}", summary(&mut route_ms));
    println!("document copy: {}", summary(&mut clone_ms));
    println!("compose CPU: {}", summary(&mut cpu));
    println!("compose + GPU: {}", summary(&mut wall));
    println!("draw reuse: {:?}", composer.renderer().draw_reuse_stats());
    if let Some(mode) = &edit {
        println!("edits ({mode}, one per frame): {apply_summary}");
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
