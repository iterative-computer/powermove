//! Phase 0 spike: render the product scenes at every tier and measure them.
//!
//!   studio-spike --out DIR [--scene hero|edge|macro|back|chrome|all]
//!                [--finish space-gray|starlight|blue|purple]
//!                [--size 1920x1080] [--final-spp 256] [--frames 0]
//!
//! Writes `<scene>-live.png`, `<scene>-refine.png`, `<scene>-final.png`,
//! optional turntable frames `<scene>-NNNN.png`, and `report.json`.

use pm_studio::gpu::{Renderer, write_png};
use pm_studio::scene::{self, preset};
use std::path::PathBuf;
use std::time::Instant;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "studio-out".into()));
    std::fs::create_dir_all(&out).expect("output directory");
    let finish = arg(&args, "--finish").unwrap_or_else(|| "space-gray".into());
    let (w, h) = arg(&args, "--size")
        .and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1920), b.parse().unwrap_or(1080))))
        .unwrap_or((1920u32, 1080u32));
    let final_spp: u32 = arg(&args, "--final-spp").and_then(|s| s.parse().ok()).unwrap_or(256);
    let frames: u32 = arg(&args, "--frames").and_then(|s| s.parse().ok()).unwrap_or(0);
    let frame_spp: u32 = arg(&args, "--frame-spp").and_then(|s| s.parse().ok()).unwrap_or(64);
    let scenes: Vec<String> = match arg(&args, "--scene").as_deref() {
        None | Some("all") => ["hero", "edge", "macro", "back", "chrome"].iter().map(|s| s.to_string()).collect(),
        Some(s) => vec![s.to_string()],
    };

    let mut r = Renderer::new(w, h).expect("GPU");
    // Live renders at half resolution and is upscaled for display.
    let mut live_r = Renderer::new(w / 2, h / 2).expect("GPU");
    live_r.live = true;
    let (sw, sh) = (1024u32, 1488u32);
    r.set_screen(&scene::screen_wallpaper(sw, sh, 0.0), sw, sh);
    live_r.set_screen(&scene::screen_wallpaper(sw, sh, 0.0), sw, sh);
    eprintln!("adapter: {} · {}×{}", r.adapter, w, h);
    let mut report = Vec::new();

    for name in &scenes {
        let t0 = Instant::now();
        let s = preset(name, &finish, 0.0).expect("scene");
        let prepared = s.prepare();
        let prep_ms = t0.elapsed().as_secs_f64() * 1000.0;
        r.set_scene(&s, &prepared);
        live_r.set_scene(&s, &prepared);

        // Warm up (pipeline creation, first dispatch), then Live: half
        // resolution, 1 spp, 3 bounces, denoised. Timed without readback.
        live_r.trace(1, 2, true, 6);
        live_r.finish(4);
        let mut live = Vec::new();
        for _ in 0..30 {
            let tr = live_r.trace(1, 2, true, 6);
            live.push(tr + live_r.post_only(4));
        }
        live.sort_by(|a, b| a.partial_cmp(b).unwrap());
        live_r.trace(1, 2, true, 6);
        let (live_img, _) = live_r.finish(4);
        write_png(&out.join(format!("{name}-live.png")), w / 2, h / 2, &live_img).unwrap();

        // Refine: accumulate in 2 ms-ish slices of 1 spp; report time to 64 spp.
        r.set_camera(&s);
        let mut refine_ms = 0.0;
        let mut first = true;
        while r.samples < 64 {
            refine_ms += r.trace(1, 6, first, 64);
            first = false;
        }
        let (px, _) = r.finish(0);
        write_png(&out.join(format!("{name}-refine.png")), w, h, &px).unwrap();

        // Final: a fixed sample count, 8 bounces.
        let mut final_ms = r.trace(final_spp, 8, true, 64);
        let (px, post) = r.finish(0);
        final_ms += post;
        write_png(&out.join(format!("{name}-final.png")), w, h, &px).unwrap();

        eprintln!(
            "{name:7} {:>7} tris · prep {prep_ms:6.1} ms · Live median {:5.2} ms p95 {:5.2} ms · Refine 64 spp {:6.0} ms · Final {final_spp} spp {:6.0} ms",
            s.triangle_count(),
            live[live.len() / 2],
            live[(live.len() * 95 / 100).min(live.len() - 1)],
            refine_ms,
            final_ms
        );
        report.push(serde_json::json!({
            "scene": name, "triangles": s.triangle_count(), "prepare_ms": prep_ms,
            "live_ms_median": live[live.len() / 2], "live_ms_p95": live[(live.len() * 95 / 100).min(live.len() - 1)],
            "refine_64spp_ms": refine_ms, "final_spp": final_spp, "final_ms": final_ms,
        }));

        // Turntable / animation frames at the Final tier with `frame_spp`.
        for f in 0..frames {
            let t = f as f32 / frames as f32;
            let s = preset(name, &finish, t).unwrap();
            let prepared = s.prepare();
            r.set_scene(&s, &prepared);
            r.set_screen(&scene::screen_wallpaper(sw, sh, t), sw, sh);
            r.trace(frame_spp, 6, true, if frame_spp < 16 { 6 } else { 64 });
            let (px, _) = r.finish(if frame_spp < 16 { 4 } else { 0 });
            write_png(&out.join(format!("{name}-{f:04}.png")), w, h, &px).unwrap();
        }
        if frames > 0 {
            r.set_screen(&scene::screen_wallpaper(sw, sh, 0.0), sw, sh);
        }
    }
    let doc = serde_json::json!({ "adapter": r.adapter, "width": w, "height": h, "scenes": report });
    std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&doc).unwrap()).unwrap();
}
