//! Repeatable corpus runner. Outputs are evidence, never repository assets.
use pm_studio::{
    corpus,
    gpu::{Renderer, write_png},
};
use std::{path::PathBuf, time::Instant};
fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|s| s == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let out =
        PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "/tmp/pm-shots/studio3d/laneC".into()));
    std::fs::create_dir_all(&out).unwrap();
    let name = arg(&args, "--scene").unwrap_or_else(|| "env-room".into());
    let scene = corpus::scene(&name).expect("environment scene name");
    std::fs::write(
        out.join(format!("{name}-scene.json")),
        serde_json::to_string(&scene).unwrap(),
    )
    .unwrap();
    if args.iter().any(|s| s == "--bake-sky") {
        if let Some(sky) = scene.world.sky {
            let map = sky.bake(256, 128);
            let data: Vec<f32> = map.pixels.iter().flat_map(|p| p.to_array()).collect();
            let image = image::Rgb32FImage::from_raw(map.width, map.height, data).unwrap();
            image
                .save_with_format(
                    out.join(format!("{name}-sky.exr")),
                    image::ImageFormat::OpenExr,
                )
                .unwrap();
            std::fs::write(
                out.join(format!("{name}-sun.json")),
                serde_json::to_string(&sky.sun()).unwrap(),
            )
            .unwrap();
        }
    }
    if args.iter().any(|s| s == "--json-only") {
        return;
    }
    let size = arg(&args, "--size").unwrap_or_else(|| "960x540".into());
    let (w, h) = size
        .split_once('x')
        .map(|(a, b)| (a.parse::<u32>().unwrap(), b.parse::<u32>().unwrap()))
        .unwrap();
    let spp = arg(&args, "--spp")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(64);
    let start = Instant::now();
    let prepared = scene.prepare();
    let prepare_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    let mut r = Renderer::new(w, h).expect("GPU");
    r.hardware_rt = !args.iter().any(|s| s == "--no-rt");
    r.set_scene(&scene, &prepared);
    let upload_ms = start.elapsed().as_secs_f64() * 1000.;
    r.live = false;
    let start = Instant::now();
    let trace_ms = r.trace(spp, 8, true, 1000000);
    let (pixels, post_ms) = r.finish(0);
    let total_seconds = start.elapsed().as_secs_f64();
    write_png(&out.join(format!("{name}-final{spp}.png")), w, h, &pixels).unwrap();
    let bytes = prepared.positions.len() * 16
        + prepared.tris.len() * 96
        + prepared.nodes.len() * 64
        + prepared.materials.len() * 128
        + prepared.sdata.len() * 16;
    let instances = prepared
        .instancing
        .as_ref()
        .map_or(1, |i| i.placements.len());
    let result = serde_json::json!({"scene":name,"size":[w,h],"samples":spp,"prepareMs":prepare_ms,"uploadMs":upload_ms,"traceMs":trace_ms,"postMs":post_ms,"totalSeconds":total_seconds,"uniqueTriangles":prepared.tris.len(),"instances":instances,"lights":prepared.lights.len(),"preparedMiB":bytes as f64/1048576.,"adapter":r.adapter,"hardwareRt":r.uses_hardware_rt()});
    println!("{result}");
    std::fs::write(
        out.join(format!("{name}-final-metrics.json")),
        serde_json::to_string_pretty(&result).unwrap(),
    )
    .unwrap();
}
