//! S4 rendering half: glyph and path rendering through Vello (0.11, GPU
//! compute), `vello_gpu` (0.3), `vello_cpu` (0.3), and Skia Graphite on
//! Metal into half-float surfaces, compared with CoreText and today's
//! Canvas2D output (SSIM), plus 4K stress timings.
//!
//! s4-render scenes            -> out/scenes.json (for `bun capture/run.ts render`)
//! s4-render compare [--only ID] -> out/render/*.png, out/render-compare.json
//! s4-render stress [--frames N] -> out/render-stress.json
//! s4-render fit                 -> out/render-fit.json (stem darkening and glyph snapping vs CoreText and Canvas2D)

mod coretext;
mod image;
mod prepare;
mod scene;
mod skia;
mod vello_r;

use image::{Image, compare};
use prepare::{Prepared, Preparer};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `+cg` variants use the CoreGraphics-smoothing tune, `+canvas` the
/// Canvas2D tune (vello_r::Tune).
const RENDERERS: &[&str] = &[
    "coretext", "coretext-smooth", "canvas2d", "vello", "vello_gpu", "vello_cpu", "skia-graphite", "vello+cg", "vello_gpu+cg", "vello_cpu+cg", "vello+canvas", "vello_gpu+canvas", "vello_gpu+cg+atlas",
];

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let root = PathBuf::from(arg("--root").unwrap_or_else(|| "out".into()));
    std::fs::create_dir_all(&root).unwrap();
    let file = scene::scenes();
    std::fs::write(root.join("scenes.json"), serde_json::to_string(&file).unwrap()).unwrap();
    match cmd.as_str() {
        "scenes" => println!("wrote {} scenes and {} stress scenes to {}", file.scenes.len(), file.stress.len(), root.join("scenes.json").display()),
        "compare" => run_compare(&root, &file),
        "stress" => run_stress(&root, &file),
        "fit" => run_fit(&root, &file),
        _ => {
            eprintln!("usage: s4-render scenes|compare|stress [--root out] [--only ID] [--frames N]");
            std::process::exit(2);
        }
    }
}

fn load_canvas(root: &Path, p: &Prepared) -> Option<Image> {
    let bytes = std::fs::read(root.join("render-canvas2d").join(format!("{}.rgba", p.id))).ok()?;
    if bytes.len() != (p.width * p.height * 4) as usize {
        eprintln!("{}: Canvas2D capture has the wrong size; skipped", p.id);
        return None;
    }
    // getImageData returns unpremultiplied RGBA.
    Some(Image::from_rgba8(p.width, p.height, &bytes, false))
}

struct Backends {
    gpu: vello_r::Gpu,
    ct: coretext::CoreTextRenderer,
    ct_smooth: coretext::CoreTextRenderer,
    vello: vello_r::VelloClassic,
    vgpu: vello_r::VelloGpu,
    vcpu: vello_r::VelloCpu,
    skia: skia::SkiaGraphite,
    vello_cg: vello_r::VelloClassic,
    vgpu_cg: vello_r::VelloGpu,
    vcpu_cg: vello_r::VelloCpu,
    vello_cv: vello_r::VelloClassic,
    vgpu_cv: vello_r::VelloGpu,
    vgpu_atlas: vello_r::VelloGpu,
}

impl Backends {
    fn new() -> Self {
        let t = std::time::Instant::now();
        let gpu = vello_r::Gpu::new();
        let t_gpu = t.elapsed();
        let t = std::time::Instant::now();
        let vello = vello_r::VelloClassic::new(&gpu);
        let t_vello = t.elapsed();
        let t = std::time::Instant::now();
        let vgpu = vello_r::VelloGpu::new(&gpu, 64, 64);
        let t_vgpu = t.elapsed();
        let t = std::time::Instant::now();
        let skia = skia::SkiaGraphite::new();
        let t_skia = t.elapsed();
        println!(
            "init: wgpu device {:.1} ms, vello renderer {:.1} ms, vello_gpu renderer {:.1} ms, Skia Graphite context {:.1} ms",
            t_gpu.as_secs_f64() * 1e3,
            t_vello.as_secs_f64() * 1e3,
            t_vgpu.as_secs_f64() * 1e3,
            t_skia.as_secs_f64() * 1e3
        );
        let tuned = |t: vello_r::Tune| {
            let mut v = vello_r::VelloClassic::new(&gpu);
            v.fonts.tune = t;
            let mut g = vello_r::VelloGpu::new(&gpu, 64, 64);
            g.fonts.tune = t;
            let mut c = vello_r::VelloCpu::new(64, 64);
            c.fonts.tune = t;
            (v, g, c)
        };
        let (vello_cg, vgpu_cg, vcpu_cg) = tuned(vello_r::Tune::COREGRAPHICS);
        let (vello_cv, vgpu_cv, _) = tuned(vello_r::Tune::CANVAS2D);
        let mut vgpu_atlas = vello_r::VelloGpu::new(&gpu, 64, 64);
        vgpu_atlas.fonts.tune = vello_r::Tune::COREGRAPHICS;
        vgpu_atlas.atlas = true;
        Backends {
            vgpu_atlas,
            vello_cg,
            vgpu_cg,
            vcpu_cg,
            vello_cv,
            vgpu_cv,
            ct: coretext::CoreTextRenderer::new(false),
            ct_smooth: coretext::CoreTextRenderer::new(true),
            vello,
            vgpu,
            vcpu: vello_r::VelloCpu::new(64, 64),
            skia,
            gpu,
        }
    }

    fn render(&mut self, name: &str, p: &Prepared, faces: &BTreeMap<u32, prepare::FaceData>) -> Image {
        match name {
            "coretext" => self.ct.render(p, faces),
            "coretext-smooth" => self.ct_smooth.render(p, faces),
            "vello" => self.vello.render(&self.gpu, p, faces),
            "vello_gpu" => self.vgpu.render(&self.gpu, p, faces),
            "vello_cpu" => self.vcpu.render(p, faces),
            "skia-graphite" => self.skia.render(p, faces),
            "vello+cg" => self.vello_cg.render(&self.gpu, p, faces),
            "vello_gpu+cg" => self.vgpu_cg.render(&self.gpu, p, faces),
            "vello_cpu+cg" => self.vcpu_cg.render(p, faces),
            "vello+canvas" => self.vello_cv.render(&self.gpu, p, faces),
            "vello_gpu+canvas" => self.vgpu_cv.render(&self.gpu, p, faces),
            "vello_gpu+cg+atlas" => self.vgpu_atlas.render(&self.gpu, p, faces),
            _ => unreachable!(),
        }
    }

    fn frame(&mut self, name: &str, p: &Prepared, faces: &BTreeMap<u32, prepare::FaceData>) -> vello_r::FrameTime {
        match name {
            "coretext" => {
                let started = std::time::Instant::now();
                self.ct.render(p, faces);
                let t = started.elapsed();
                vello_r::FrameTime { encode: t, total: t }
            }
            "vello" => self.vello.frame(&self.gpu, p, faces),
            "vello_gpu" => self.vgpu.frame(&self.gpu, p, faces),
            "vello_cpu" => self.vcpu.frame(p, faces),
            "skia-graphite" => self.skia.frame(p, faces),
            "vello+cg" => self.vello_cg.frame(&self.gpu, p, faces),
            "vello_gpu+cg" => self.vgpu_cg.frame(&self.gpu, p, faces),
            "vello_cpu+cg" => self.vcpu_cg.frame(p, faces),
            "vello_gpu+cg+atlas" => self.vgpu_atlas.frame(&self.gpu, p, faces),
            "coretext-smooth" => {
                let started = std::time::Instant::now();
                self.ct_smooth.render(p, faces);
                let t = started.elapsed();
                vello_r::FrameTime { encode: t, total: t }
            }
            _ => unreachable!(),
        }
    }
}

fn run_compare(root: &Path, file: &scene::SceneFile) {
    let only = arg("--only");
    let mut prep = Preparer::new();
    let prepared: Vec<Prepared> = file.scenes.iter().filter(|s| only.as_ref().is_none_or(|o| &s.id == o)).map(|s| prep.prepare(s)).collect();
    let synthetic: Vec<String> = prep.faces.values().filter(|f| f.synthetic_bold || f.synthetic_italic).map(|f| f.postscript_name.clone()).collect();
    let mut b = Backends::new();
    let out_dir = root.join("render");
    std::fs::create_dir_all(&out_dir).unwrap();
    let mut images: BTreeMap<String, BTreeMap<&str, Image>> = BTreeMap::new();
    for p in &prepared {
        let mut per = BTreeMap::new();
        for &r in RENDERERS {
            if r == "canvas2d" {
                if let Some(img) = load_canvas(root, p) {
                    per.insert(r, img);
                }
                continue;
            }
            per.insert(r, b.render(r, p, &prep.faces));
        }
        let rows: Vec<&Image> = RENDERERS.iter().filter_map(|r| per.get(r)).collect();
        Image::stack(&rows).write_png(&out_dir.join(format!("{}.png", p.id)), Some([0.0, 0.0, 0.0]));
        images.insert(p.id.clone(), per);
    }

    // Every renderer against each reference, per scene.
    let refs = ["coretext", "coretext-smooth", "canvas2d"];
    let mut scenes_json = serde_json::Map::new();
    for p in &prepared {
        let per = &images[&p.id];
        let mut by_ref = serde_json::Map::new();
        for r in refs {
            let Some(reference) = per.get(r) else { continue };
            let mut row = serde_json::Map::new();
            for &t in RENDERERS {
                if t == r {
                    continue;
                }
                if let Some(img) = per.get(t) {
                    row.insert(t.into(), serde_json::to_value(compare(img, reference)).unwrap());
                }
            }
            by_ref.insert(r.into(), Value::Object(row));
        }
        scenes_json.insert(p.id.clone(), json!({ "group": p.group, "width": p.width, "height": p.height, "glyphs": p.glyphs, "pathSegments": p.path_segments, "vs": by_ref }));
    }

    // Summary: min and mean alpha SSIM (ink windows) per renderer and group.
    let mut summary = serde_json::Map::new();
    println!("alpha SSIM over ink windows (min / mean) by reference and group:");
    for r in refs {
        let mut per_ref = serde_json::Map::new();
        for group in ["text", "path", "fit"] {
            for &t in RENDERERS {
                if t == r {
                    continue;
                }
                let vals: Vec<(f64, f64, f64)> = prepared
                    .iter()
                    .filter(|p| p.group == group)
                    .filter_map(|p| {
                        let c = scenes_json[&p.id]["vs"].get(r)?.get(t)?;
                        Some((c["alpha"]["ink"].as_f64()?, c["alpha"]["full"].as_f64()?, c["inkRatio"].as_f64()?))
                    })
                    .collect();
                if vals.is_empty() {
                    continue;
                }
                let min = vals.iter().map(|v| v.0).fold(f64::INFINITY, f64::min);
                let mean = vals.iter().map(|v| v.0).sum::<f64>() / vals.len() as f64;
                let min_full = vals.iter().map(|v| v.1).fold(f64::INFINITY, f64::min);
                let ink = vals.iter().map(|v| v.2).sum::<f64>() / vals.len() as f64;
                println!("  vs {r:<16} {group:<5} {t:<16} ink {min:.4} / {mean:.4}   full-image min {min_full:.4}   ink ratio {ink:.3}");
                per_ref.insert(format!("{group}/{t}"), json!({ "scenes": vals.len(), "inkMin": min, "inkMean": mean, "fullMin": min_full, "inkRatioMean": ink }));
            }
        }
        summary.insert(r.into(), Value::Object(per_ref));
    }

    // Coverage gamma fit for the renderers whose coverage the engine owns
    // (coverage-then-paint): alpha' = alpha^γ, maximizing the mean ink-window
    // alpha SSIM over the text scenes.
    let mut gamma = serde_json::Map::new();
    for r in ["coretext", "coretext-smooth", "canvas2d"] {
        let mut per_ref = serde_json::Map::new();
        for t in ["vello", "vello_gpu", "vello_cpu", "skia-graphite"] {
            let pairs: Vec<(&Image, &Image)> =
                prepared.iter().filter(|p| p.group == "text").filter_map(|p| Some((images[&p.id].get(t)?, images[&p.id].get(r)?))).collect();
            if pairs.is_empty() {
                continue;
            }
            let score = |g: f32| {
                let v: Vec<f64> = pairs.iter().map(|(a, b)| image::ssim(&a.with_coverage_gamma(g).channel(3), &b.channel(3), a.width as usize, a.height as usize).ink).collect();
                (v.iter().sum::<f64>() / v.len() as f64, v.iter().copied().fold(f64::INFINITY, f64::min))
            };
            let base = score(1.0);
            let mut best = (1.0f32, base);
            let mut g = 0.40f32;
            while g <= 1.601 {
                let s = score(g);
                if s.0 > best.1.0 {
                    best = (g, s);
                }
                g += 0.05;
            }
            println!("  gamma vs {r:<16} {t:<14} γ=1: mean {:.4} min {:.4}   best γ={:.2}: mean {:.4} min {:.4}", base.0, base.1, best.0, best.1.0, best.1.1);
            per_ref.insert(t.into(), json!({ "gamma1": { "mean": base.0, "min": base.1 }, "bestGamma": best.0, "best": { "mean": best.1.0, "min": best.1.1 } }));
        }
        gamma.insert(r.into(), Value::Object(per_ref));
    }

    let report = json!({
        "adapter": b.gpu.adapter,
        "syntheticFaces": synthetic,
        "faces": prep.faces.values().map(|f| f.postscript_name.clone()).collect::<Vec<_>>(),
        "summary": summary,
        "coverageGamma": gamma,
        "scenes": scenes_json,
    });
    std::fs::write(root.join("render-compare.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("wrote {} and review sheets in {} (rows: {})", root.join("render-compare.json").display(), out_dir.display(), RENDERERS.join(", "));
}

fn stats(v: &[Duration]) -> Value {
    let mut ms: Vec<f64> = v.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pick = |q: f64| ms[((ms.len() - 1) as f64 * q).round() as usize];
    json!({ "median": pick(0.5), "p95": pick(0.95), "max": ms[ms.len() - 1], "min": ms[0] })
}

fn run_stress(root: &Path, file: &scene::SceneFile) {
    let frames: usize = arg("--frames").and_then(|f| f.parse().ok()).unwrap_or(file.stress_frames as usize);
    let mut prep = Preparer::new();
    let prepared: Vec<Prepared> = file.stress.iter().map(|s| prep.prepare(s)).collect();
    let mut b = Backends::new();
    let canvas_manifest: Option<Value> = std::fs::read_to_string(root.join("render-canvas2d").join("manifest.json")).ok().and_then(|s| serde_json::from_str(&s).ok());
    let mut out = serde_json::Map::new();
    for p in &prepared {
        println!("{}: {}x{}, {} glyphs, {} path elements", p.id, p.width, p.height, p.glyphs, p.path_segments);
        let mut per = serde_json::Map::new();
        let plain = b.render("coretext", p, &prep.faces);
        let smooth = b.render("coretext-smooth", p, &prep.faces);
        for r in ["vello", "vello_gpu", "vello_cpu", "vello+cg", "vello_gpu+cg", "vello_cpu+cg", "vello_gpu+cg+atlas", "skia-graphite", "coretext", "coretext-smooth"] {
            // The first frame includes glyph outline and atlas caches.
            let first = b.frame(r, p, &prep.faces);
            let mut enc = Vec::new();
            let mut tot = Vec::new();
            for f in 1..=frames {
                let shifted = p.shifted((f as f64 * 0.137) % 1.0);
                let t = b.frame(r, &shifted, &prep.faces);
                enc.push(t.encode);
                tot.push(t.total);
            }
            let img = b.render(r, p, &prep.faces);
            // Tuned renderers aim at CoreText with font smoothing; plain ones at plain CoreText.
            let (rname, refimg) = if r.contains("+cg") || r == "coretext-smooth" || r == "skia-graphite" { ("coretext-smooth", &smooth) } else { ("coretext", &plain) };
            let cmp = compare(&img, refimg);
            let st = stats(&tot);
            println!(
                "  {r:<19} first {:>8.2} ms   frame median {:>8.2} ms p95 {:>8.2} ms (CPU encode median {:>7.2} ms)   alpha SSIM vs {rname} ink {:.4}",
                first.total.as_secs_f64() * 1000.0,
                st["median"].as_f64().unwrap(),
                st["p95"].as_f64().unwrap(),
                stats(&enc)["median"].as_f64().unwrap(),
                cmp.alpha.ink
            );
            per.insert(r.into(), json!({ "firstMs": first.total.as_secs_f64() * 1000.0, "frameMs": st, "encodeMs": stats(&enc), "reference": rname, "vsReference": cmp }));
        }
        if let Some(m) = canvas_manifest.as_ref().and_then(|m| m["stress"].as_array()).and_then(|a| a.iter().find(|s| s["id"] == p.id.as_str())) {
            let ms: Vec<Duration> = m["frameMs"].as_array().unwrap().iter().map(|v| Duration::from_secs_f64(v.as_f64().unwrap() / 1000.0)).collect();
            let canvas_img = load_canvas(root, p);
            let st = stats(&ms);
            let cmp = canvas_img.map(|c| compare(&c, &smooth));
            println!("  {:<19} first {:>8.2} ms   frame median {:>8.2} ms p95 {:>8.2} ms (draw + WebGL texture upload + 1 px readPixels, Electron)", "canvas2d", m["firstMs"].as_f64().unwrap(), st["median"].as_f64().unwrap(), st["p95"].as_f64().unwrap());
            per.insert("canvas2d".into(), json!({ "firstMs": m["firstMs"], "frameMs": st, "reference": "coretext-smooth", "vsReference": cmp }));
        }
        out.insert(p.id.clone(), json!({ "width": p.width, "height": p.height, "glyphs": p.glyphs, "pathElements": p.path_segments, "frames": frames, "renderers": per }));
    }
    let report = json!({ "adapter": b.gpu.adapter, "scenes": out });
    std::fs::write(root.join("render-stress.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
}

/// Fits Vello's stem darkening (outline expansion in px per side) and
/// glyph position snapping per size against CoreText with font smoothing
/// and against today's Canvas2D, on the `fit-*` scenes.
fn run_fit(root: &Path, file: &scene::SceneFile) {
    let mut prep = Preparer::new();
    let gpu = vello_r::Gpu::new();
    let mut vello = vello_r::VelloClassic::new(&gpu);
    let mut ct = coretext::CoreTextRenderer::new(true);
    let mut out = Vec::new();
    for s in file.scenes.iter().filter(|s| s.group == "fit") {
        let p = prep.prepare(s);
        let size = s.items.iter().find_map(|i| if let scene::Item::Text { d, .. } = i { d["size"].as_f64() } else { None }).unwrap();
        let refs: Vec<(&str, Image)> = [("coretext-smooth", Some(ct.render(&p, &prep.faces))), ("canvas2d", load_canvas(root, &p))].into_iter().filter_map(|(n, i)| Some((n, i?))).collect();
        let mut rows = serde_json::Map::new();
        for (rname, reference) in &refs {
            let alpha = reference.channel(3);
            for snap in [vello_r::Snap::Exact, vello_r::Snap::QuarterRound] {
                let mut score = |dilation: f64, dy: f64| {
                    vello.fonts.tune = vello_r::Tune::fixed(dilation, dy, snap);
                    let img = vello.render(&gpu, &p, &prep.faces);
                    image::ssim(&img.channel(3), &alpha, p.width as usize, p.height as usize).ink
                };
                let plain = score(0.0, 0.0);
                let mut best = (0.0f64, 0.0f64, f64::MIN);
                for di in 0..=25 {
                    for oi in 0..=10 {
                        let (d, o) = (di as f64 * 0.02, -0.4 + oi as f64 * 0.05);
                        let v = score(d, o);
                        if v > best.2 {
                            best = (d, o, v);
                        }
                    }
                }
                println!("size {size:>5} vs {rname:<16} {:<13} plain {plain:.4}  best dilation {:.2} px, dy {:+.2}: {:.4}", format!("{snap:?}"), best.0, best.1, best.2);
                rows.insert(format!("{rname}/{snap:?}"), json!({ "plain": plain, "bestDilation": best.0, "bestDy": best.1, "bestSsim": best.2 }));
            }
        }
        out.push(json!({ "size": size, "scene": s.id, "fits": rows }));
    }
    std::fs::write(root.join("render-fit.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
}
