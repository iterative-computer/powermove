//! Text raster parity against today's Canvas2D (plan section 25, text
//! class; amendment 10): every case of the rendering corpus
//! (`pm-text/capture/render-cases.ts`: sizes 8 to 200 px, families, weights,
//! italics, synthetic faces, variable fonts, emoji, RTL and complex
//! scripts, tracking, paragraphs, colors, raster densities, every caption
//! preset and word state, stroked captions) is drawn through the production
//! path (`Renderer::text_raster`: pm-eval geometry, pm-text level 1 layout,
//! Vello coverage, the float paint pass) and compared with the canvas the
//! legacy code painted, as the compositor uploads it (premultiplied RGBA8).
//!
//! Score: SSIM (the mean of the four channels' maps, 11 × 11 Gaussian,
//! σ = 1.5) over ink windows (pixels whose local mean alpha exceeds 1/512 in
//! either image; empty background cannot inflate it), and over the full
//! canvas. Gate: ink SSIM ≥ 0.98 for every case, except the cases pinned in
//! `tests/fixtures/text-raster-pins.json` (known to be below the class,
//! docs/engine/text-render.md "Gaps"), which must stay above their floors.
//!
//! The capture is private (renders of the system fonts): `PM_TEXT_RASTER`
//! names it, else the newest `text-raster-*` in the corpus directory.
//! Without one the test reports and passes vacuously. Capture with
//! `pm-electron-lock bun engine/crates/pm-text/capture/render.ts`.
//!
//! `PM_TEXT_ONLY=<prefix>` limits cases; `PM_TEXT_DIFFS=<dir>` writes
//! reference / engine / difference PNGs of failing cases;
//! `PM_TEXT_PROFILE=<json>` overrides the level 1 profile (fitting);
//! `PM_TEXT_COVERAGE=cpu` uses the `vello_cpu` fallback;
//! `PM_TEXT_REPORT=<file>` writes every case's scores as JSON.
//!
//! Runs without the test harness so the CoreText registry lives on the main
//! thread.

mod support;

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_render::Renderer;
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use support::doc::{layer, project};
    use support::text::{ink_ssim, png_rgba};

    let dir = match std::env::var("PM_TEXT_RASTER") {
        Ok(p) => Some(PathBuf::from(p)),
        Err(_) => support::newest_capture("text-raster-"),
    };
    let Some(dir) = dir else {
        eprintln!(
            "no text raster capture; set PM_TEXT_RASTER or capture with pm-text/capture/render.ts"
        );
        return;
    };
    let cases: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let corpus = pm_text::corpus::CorpusFile::from_value(&json!({
        "variableSources": cases["variableSources"],
        "projectFonts": cases["projectFonts"],
        "cases": [],
    }))
    .unwrap();
    let registry = pm_text::corpus::native_registry(&corpus).expect("registry");
    let mut renderer = Renderer::headless().expect("a Metal device");
    renderer.set_text_fonts(Some(Box::new(support::text::MainThreadFonts(
        pm_eval::text_measure::CompatMeasure::new(registry),
        None,
    ))));
    if let Ok(p) = std::env::var("PM_TEXT_PROFILE") {
        let profile: pm_text::render::profile::Profile =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or(p)).expect("profile json");
        renderer.text().set_profile(profile);
    }
    // `PM_TEXT_OUTLINES=1`: the outline path with the Canvas2D profile (no
    // platform masks), for comparison.
    if std::env::var("PM_TEXT_OUTLINES").as_deref() == Ok("1") {
        renderer.text().set_masks(None);
    }
    if std::env::var("PM_TEXT_COVERAGE").as_deref() == Ok("cpu") {
        renderer
            .text_gpu()
            .set_coverage(pm_render::text::gpu::Coverage::Cpu);
    }
    let only = std::env::var("PM_TEXT_ONLY").ok();
    let diffs = std::env::var("PM_TEXT_DIFFS").ok().map(PathBuf::from);
    // Known cases below the text class and their floors (docs/engine/text-render.md, Gaps).
    let pins: std::collections::BTreeMap<String, f64> = {
        let v: Value =
            serde_json::from_str(include_str!("fixtures/text-raster-pins.json")).unwrap();
        v["pins"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_f64().unwrap()))
            .collect()
    };
    let mut pinned_below = 0;
    let mut rows = Vec::new();
    let mut failed = Vec::new();
    let started = std::time::Instant::now();
    for (case, shot) in cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(manifest["cases"].as_array().unwrap())
    {
        let id = case["id"].as_str().unwrap();
        assert_eq!(shot["id"].as_str(), Some(id));
        if only.as_deref().is_some_and(|o| !id.starts_with(o)) {
            continue;
        }
        if shot.get("empty").is_some() {
            continue;
        }
        let (doc, comp, layer_id, time) = if case["kind"] == "text" {
            let l = layer("t", "text", case["d"].clone(), 0.0, 0.0, 10.0);
            (
                project(1920, 1080, 10.0, support::doc::no_fill(), vec![l]),
                "Ptest-root",
                "t",
                0.0,
            )
        } else {
            let l = layer("c", "captions", case["content"].clone(), 960.0, 540.0, 10.0);
            let w = case["compositionWidth"].as_u64().unwrap() as u32;
            let h = case["compositionHeight"].as_u64().unwrap() as u32;
            (
                project(w, h, 10.0, support::doc::no_fill(), vec![l]),
                "Ptest-root",
                "c",
                case["local"].as_f64().unwrap(),
            )
        };
        let document =
            pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1);
        let scale = case["scale"].as_f64().unwrap();
        let (cw, ch) = (
            shot["width"].as_u64().unwrap() as usize,
            shot["height"].as_u64().unwrap() as usize,
        );
        let reference = std::fs::read(dir.join(format!("{id}.rgba"))).unwrap();
        let raster = renderer
            .text_raster(&document, comp, layer_id, time, scale, Precision::Float16)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        let mut ours = vec![0u8; cw * ch * 4];
        let mut geometry = String::new();
        if let Some(r) = &raster {
            if (r.canvas.0 as usize, r.canvas.1 as usize) != (cw, ch) {
                geometry = format!("canvas {}x{} vs {cw}x{ch}", r.canvas.0, r.canvas.1);
            }
            let [x0, y0, x1, y1] = r.plan.crop.map(|v| v as usize);
            let w = x1 - x0;
            for y in y0..y1.min(ch) {
                for x in x0..x1.min(cw) {
                    for c in 0..4 {
                        let v = r.pixels[((y - y0) * w + (x - x0)) * 4 + c];
                        ours[(y * cw + x) * 4 + c] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        let (ink, full, ratio) = ink_ssim(&reference, &ours, cw, ch);
        let pin = pins.get(id).copied();
        let pass = geometry.is_empty()
            && match pin {
                Some(floor) => ink >= floor,
                None => ink >= 0.98,
            };
        if pin.is_some() && ink < 0.98 {
            pinned_below += 1;
        }
        // The largest channel difference, and the share of pixels off by
        // more than one code value (exactness, not only the text class).
        let (mut max_diff, mut off) = (0u8, 0usize);
        for (a, b) in reference.chunks_exact(4).zip(ours.chunks_exact(4)) {
            let d = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap();
            max_diff = max_diff.max(d);
            off += usize::from(d > 1);
        }
        let line = format!(
            "{id:<26} {:>5}x{:<5} ink SSIM {ink:.4}  full {full:.4}  ink ratio {ratio:.3}  max Δ {max_diff:>3}  >1 CV {off:>6} {geometry}",
            cw, ch
        );
        println!("{line}");
        if !pass {
            failed.push(line);
            if let Some(d) = &diffs {
                std::fs::create_dir_all(d).unwrap();
                let diff: Vec<u8> = reference
                    .chunks_exact(4)
                    .zip(ours.chunks_exact(4))
                    .flat_map(|(a, b)| {
                        let e = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap();
                        [e.saturating_mul(4), 0, 0, 255]
                    })
                    .collect();
                for (name, px) in [("legacy", &reference), ("engine", &ours), ("diff", &diff)] {
                    support::write_png(
                        &d.join(format!("{id}-{name}.png")),
                        cw as u32,
                        ch as u32,
                        &png_rgba(px, name != "diff"),
                    );
                }
            }
        }
        // Raw premultiplied buffers of every case, for fitting tools.
        if let Ok(d) = std::env::var("PM_TEXT_DUMP") {
            let d = PathBuf::from(d);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(format!("{id}.{cw}x{ch}.engine.rgba")), &ours).unwrap();
            std::fs::write(d.join(format!("{id}.{cw}x{ch}.legacy.rgba")), &reference).unwrap();
        }
        rows.push(json!({"id": id, "group": case["group"], "inkSsim": ink, "fullSsim": full, "inkRatio": ratio, "maxDiff": max_diff, "over1": off, "pass": pass}));
    }
    // Per group summary.
    let mut groups: std::collections::BTreeMap<String, (usize, f64, f64)> = Default::default();
    for r in &rows {
        let g = groups
            .entry(r["group"].as_str().unwrap().to_owned())
            .or_insert((0, 1.0, 0.0));
        g.0 += 1;
        g.1 = g.1.min(r["inkSsim"].as_f64().unwrap());
        g.2 += r["inkSsim"].as_f64().unwrap();
    }
    println!(
        "\n{:<20} {:>5} {:>8} {:>8}",
        "group", "cases", "min", "mean"
    );
    for (g, (n, min, sum)) in &groups {
        println!("{g:<20} {n:>5} {min:>8.4} {:>8.4}", sum / *n as f64);
    }
    let stats = renderer.text_gpu().stats;
    println!(
        "\n{} cases in {:.1} s; {} pinned cases below 0.98 (gap); {} failed; pages {} (cpu {}); vello: {}",
        rows.len(),
        started.elapsed().as_secs_f64(),
        pinned_below,
        failed.len(),
        stats.pages,
        stats.cpu_pages,
        renderer.text_gpu().vello_failure().unwrap_or("ok")
    );
    if let Ok(p) = std::env::var("PM_TEXT_REPORT") {
        std::fs::write(p, serde_json::to_string_pretty(&rows).unwrap()).unwrap();
    }
    for f in &failed {
        println!("  FAILED {f}");
    }
    assert!(
        failed.is_empty(),
        "{} cases below the text class",
        failed.len()
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
