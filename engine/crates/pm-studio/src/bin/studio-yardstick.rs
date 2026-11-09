//! Test-only corpus export. Blender is invoked only by tools/cycles-yardstick.
use pm_studio::{
    environment::EnvironmentMap,
    gpu::{Renderer, write_png},
    scene,
};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

fn arg(args: &[String], name: &str, default: &str) -> String {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| default.into())
}
fn exr(path: &Path, w: u32, h: u32, pixels: Vec<f32>) {
    image::Rgba32FImage::from_raw(w, h, pixels)
        .unwrap()
        .save(path)
        .unwrap();
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = arg(&args, "--scene", "hero");
    let out = PathBuf::from(arg(&args, "--out", "/tmp/pm-shots/studio3d/laneE/hero"));
    std::fs::create_dir_all(&out).unwrap();
    let size = arg(&args, "--size", "320x180");
    let (w, h) = size.split_once('x').unwrap();
    let (w, h): (u32, u32) = (w.parse().unwrap(), h.parse().unwrap());
    assert!(
        w > 0 && h > 0 && w <= 1920 && h <= 1080,
        "bounded diagnostic resolution"
    );
    let spp: u32 = arg(&args, "--spp", "256").parse().unwrap();
    assert!((1..=4096).contains(&spp));
    // The exact edge-on pose at zero is subpixel at this diagnostic size.
    let time = if name == "edge" { 0.125 } else { 0. };
    let input = arg(&args, "--input", "");
    let source = if input.is_empty() {
        scene::preset(&name, "starlight", time).expect("corpus scene")
    } else {
        serde_json::from_slice(&std::fs::read(input).expect("scene input")).expect("scene JSON")
    };
    let mut s = source.expanded().expect("expanded geometry");
    s.world.adaptive.enabled = false;
    let start = Instant::now();
    let mut r = Renderer::new(w, h).expect("Studio GPU");
    r.hardware_rt = !args.iter().any(|a| a == "--no-rt");
    r.live = false;
    let (sw, sh) = (512, 744);
    let screen = scene::screen_wallpaper(sw, sh, 0.);
    r.set_screen(&screen, sw, sh);
    write_png(&out.join("screen.png"), sw, sh, &screen).unwrap();
    let prepared = s.prepare();
    r.set_scene(&s, &prepared);
    if args.iter().any(|a| a == "--no-caustics") {
        r.caustics = false;
    }
    // Final transport, eight bounces, no denoising. Keep the usual Final clamp
    // unless explicitly measuring an unclamped estimator.
    let clamp = arg(&args, "--clamp", "64").parse().unwrap();
    r.trace(spp, 8, true, clamp);
    let pixels = r.linear_beauty();
    exr(&out.join("studio.exr"), w, h, pixels.clone());
    let mut bytes = Vec::with_capacity(pixels.len() * 4);
    for p in pixels {
        bytes.extend(p.to_le_bytes());
    }
    std::fs::write(out.join("studio.rgba32f"), bytes).unwrap();
    let ids = r.first_hit_ids();
    std::fs::write(
        out.join("studio.ids"),
        ids.iter()
            .flat_map(|id| id.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let (display, _) = r.finish(0);
    write_png(&out.join("studio.png"), w, h, &display).unwrap();
    let map = s
        .world
        .hdri
        .clone()
        .or_else(|| s.world.sky.map(|sky| sky.bake(128, 64)))
        .unwrap_or_else(|| {
            let (mw, mh) = (256, 128);
            let pixels = (0..mh)
                .flat_map(|y| {
                    let dy = (std::f32::consts::PI * (y as f32 + 0.5) / mh as f32).cos();
                    let color = if dy < 0. {
                        s.environment.1 * (1. - 0.5 * (-dy * 4.).clamp(0., 1.))
                    } else {
                        s.environment.1 * (1. - dy) + s.environment.0 * dy
                    };
                    std::iter::repeat_n(color, mw as usize)
                })
                .collect();
            EnvironmentMap {
                width: mw,
                height: mh,
                pixels,
                ..Default::default()
            }
        });
    let data = map
        .pixels
        .iter()
        .flat_map(|p| [p.x, p.y, p.z, 1.])
        .collect();
    exr(&out.join("world.exr"), map.width, map.height, data);
    let manifest = serde_json::json!({"scene":name,"time":time,"size":[w,h],"samples":spp,"bounces":8,"clamp":clamp,"adapter":r.adapter,"seconds":start.elapsed().as_secs_f64(),"hardware_rt":r.uses_hardware_rt(),"map":{"rotation":map.rotation,"intensity":map.intensity,"visible":s.world.hdri.as_ref().map_or_else(||s.world.sky.is_some_and(|s|s.visible),|m|m.visible)},"sun":s.world.sun(),"source":"scene.rs corpus, starlight finish","floor_albedo":0.82});
    std::fs::write(
        out.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(out.join("scene.json"), serde_json::to_string(&s).unwrap()).unwrap();
    // Make layer identity explicit for region masks, including shared instances.
    let layers: Vec<_> = s.instances.iter().enumerate().map(|(i,p)|serde_json::json!({"layer":p.layer.unwrap_or_else(||s.layers.as_ref().and_then(|l|l.get(p.object).copied()).unwrap_or(p.object as u32)),"object":p.object,"placement":i})).collect();
    std::fs::write(
        out.join("placements.json"),
        serde_json::to_string(&layers).unwrap(),
    )
    .unwrap();
    eprintln!(
        "{name}: {w}x{h} {spp}spp {:.2}s",
        start.elapsed().as_secs_f64()
    );
}
