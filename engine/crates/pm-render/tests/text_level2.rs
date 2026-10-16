//! Level 2 rasterization against CoreText with font smoothing (plan section
//! 12 and amendment 10: "level 2 against CoreText with smoothing"): every
//! outline-text case of the rendering corpus (`text-raster-*`; emoji and
//! synthesized faces excepted, below) is laid out with level 1 and drawn
//! at its exact device positions twice: by the engine (Vello coverage with
//! the CoreText profile, the float paint pass) and by CoreText itself
//! (`CTFontDrawGlyphs`, smoothing on). Same glyphs, same positions, so the
//! comparison isolates rasterization. Gate: ink SSIM ≥ 0.98 per case.
//!
//! Not compared: Apple Color Emoji (CoreText's own `CTFontDrawGlyphs` sits
//! the bitmaps 6 px higher at 48 px than Canvas2D does; amendment 10 makes
//! Canvas2D placement the rule for both levels), and synthesized bold and
//! italic (the reference would need CoreText's own synthesis).
//!
//! `PM_TEXT_RASTER` picks the capture; `PM_TEXT_ONLY` filters cases.
//! Runs without the harness (CoreText registry on the main thread).

mod support;

#[cfg(target_os = "macos")]
fn main() {
    use kurbo::Affine;
    use pm_protocol::color::Precision;
    use pm_render::Renderer;
    use pm_text::compat::TextContent;
    use pm_text::render::bitmap::Bitmaps;
    use pm_text::render::coretext::{CoreTextReference, RefRun};
    use pm_text::render::outline::{FaceRef, Outlines};
    use pm_text::render::profile::Profile;
    use pm_text::render::scene::{Painter, Run, TextScene};
    use serde_json::Value;

    let dir = match std::env::var("PM_TEXT_RASTER") {
        Ok(p) => Some(std::path::PathBuf::from(p)),
        Err(_) => support::newest_capture("text-raster-"),
    };
    let Some(dir) = dir else {
        eprintln!("no text raster capture; set PM_TEXT_RASTER");
        return;
    };
    let cases: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("cases.json")).unwrap()).unwrap();
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let corpus = pm_text::corpus::CorpusFile::from_value(&serde_json::json!({
        "variableSources": cases["variableSources"], "projectFonts": cases["projectFonts"], "cases": [],
    }))
    .unwrap();
    let registry = pm_text::corpus::native_registry(&corpus).expect("registry");
    let mut fonts = support::text::MainThreadFonts(pm_eval::text_measure::CompatMeasure::new(registry));
    let mut renderer = Renderer::headless().expect("a Metal device");
    let mut reference = CoreTextReference::new();
    let (mut outlines, mut bitmaps) = (Outlines::new(), Bitmaps::new());
    let only = std::env::var("PM_TEXT_ONLY").ok();
    let profile = Profile::CORETEXT;
    let mut failed = Vec::new();
    let mut groups: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    for (case, shot) in cases["cases"].as_array().unwrap().iter().zip(manifest["cases"].as_array().unwrap()) {
        let id = case["id"].as_str().unwrap();
        let group = case["group"].as_str().unwrap();
        if case["kind"] != "text" || group == "emoji" || group == "synthetic" {
            continue;
        }
        if only.as_deref().is_some_and(|o| !id.starts_with(o)) {
            continue;
        }
        let text = case["d"]["text"].as_str().unwrap_or("");
        if text.chars().any(|c| u32::from(c) >= 0x1F000 || c == '✂' || c == '❤') {
            continue;
        }
        let g = &shot["geometry"];
        let (w, h) = (shot["width"].as_u64().unwrap() as u32, shot["height"].as_u64().unwrap() as u32);
        let density = g["density"].as_f64().unwrap();
        let (gx, pad) = (g["x"].as_f64().unwrap(), g["pad"].as_f64().unwrap());
        let tc = TextContent::from_json(&case["d"]);
        let color = pm_text::render::color::parse_or(case["d"]["color"].as_str().unwrap_or("#fff"), [0.0, 0.0, 0.0, 1.0]);
        let lines = {
            use pm_render::text::TextFonts;
            fonts.line_runs(&tc)
        };
        let faces: Vec<Vec<pm_text::font::FaceInfo>> = lines
            .iter()
            .map(|l| {
                l.runs
                    .iter()
                    .map(|r| {
                        use pm_render::text::TextFonts;
                        fonts.face(r.face).clone()
                    })
                    .collect()
            })
            .collect();
        if lines.iter().flat_map(|l| &l.runs).any(|r| r.synthetic_bold || r.synthetic_italic) {
            continue;
        }
        let mut scene = TextScene { width: w, height: h, items: Vec::new() };
        let mut painter = Painter {
            outlines: &mut outlines,
            bitmaps: &mut bitmaps,
            profile: profile.clone(),
            unsupported: Vec::new(),
        };
        let mut refs: Vec<RefRun<'_>> = Vec::new();
        for (line, faces) in lines.iter().zip(&faces) {
            let runs: Vec<Run<'_>> = line
                .runs
                .iter()
                .zip(faces)
                .map(|(r, f)| Run {
                    face: FaceRef::of(f),
                    size: r.size,
                    coords: &r.coords,
                    synthetic_bold: false,
                    synthetic_italic: false,
                    glyphs: &r.glyphs,
                    bitmap_size: None,
                })
                .collect();
            painter.fill_text(&mut scene.items, &runs, (gx, pad), Affine::scale(density), color, 1.0, 0.0);
            for (r, f) in line.runs.iter().zip(faces) {
                refs.push(RefRun {
                    data: f.data.as_slice(),
                    index: f.index,
                    size: (f64::from(r.size) * density) as f32,
                    coords: &r.coords,
                    glyphs: r
                        .glyphs
                        .iter()
                        .map(|p| (p.id as u16, (gx + p.x) * density, (pad + p.y) * density))
                        .collect(),
                    color,
                });
            }
        }
        let ours = renderer.paint_text_scene(scene, Precision::Float32).unwrap();
        let theirs = reference.render(&refs, w, h, true);
        let to8 = |p: &[f32]| -> Vec<u8> { p.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8).collect() };
        let (ink, full, ratio) = pm_render::parity::text_ssim(&to8(&theirs), &to8(&ours), w as usize, h as usize);
        println!("{id:<24} ink SSIM {ink:.4} full {full:.4} ink ratio {ratio:.3}");
        groups.entry(group.to_owned()).or_default().push(ink);
        if ink < 0.98 {
            failed.push(format!("{id} {ink:.4}"));
        }
    }
    println!("\n{:<16} {:>5} {:>8} {:>8}", "group", "cases", "min", "mean");
    let mut all: Vec<f64> = Vec::new();
    for (g, v) in &groups {
        let min = v.iter().cloned().fold(1.0, f64::min);
        println!("{g:<16} {:>5} {min:>8.4} {:>8.4}", v.len(), v.iter().sum::<f64>() / v.len() as f64);
        all.extend(v);
    }
    println!("{} cases, {} below 0.98", all.len(), failed.len());
    for f in &failed {
        println!("  FAILED {f}");
    }
    assert!(failed.is_empty(), "{} cases below 0.98 against CoreText", failed.len());
}

#[cfg(not(target_os = "macos"))]
fn main() {}
