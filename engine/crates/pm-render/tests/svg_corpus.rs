//! Level 1 SVG images in whole corpus frames against the legacy renderer
//! (decision S1; docs/engine/vector.md, "Level 1 SVG images"): every corpus
//! project with a level 1 SVG image is rendered at every captured sample
//! with Chromium's rasters of today's buckets (captured by the app's own
//! module, `tools/svg-rasters`), the engine's fonts, and the built-in
//! definitions, and compared with the capture's opaque frame.
//!
//! The *SVG pixels* are those our frame changes when its SVG image layers
//! are hidden (opacity 0), less the text class's pixels (text and captions,
//! grown by 3, as `text_corpus.rs` judges them): they must be exact. The
//! whole frame's numbers are printed beside them.
//!
//! `PM_SVG_RASTERS` names the raster directory (default: the newest
//! `svg-rasters-*` capture directory); rasters it lacks are listed in its
//! `needs.json` for `tools/svg-rasters/run.ts --needs`. `PM_ENGINE_CORPUS`
//! picks captures (default as `corpus.rs`); `PM_SVG_ONLY` limits project
//! names by prefix; `PM_SVG_DIFFS=<dir>` writes engine and legacy PNGs of
//! failures.
//!
//! Runs without the harness (the CoreText registry needs the main thread).

mod support;

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_protocol::common::Size;
    use pm_render::parity::{dilate, ssim_map};
    use pm_render::vector::svg_rasters::RasterDir;
    use pm_render::{RenderRequest, Renderer, all_builtin_definitions};
    use support::*;

    let captures = corpus_dirs();
    if captures.is_empty() {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let Some(dir) = std::env::var("PM_SVG_RASTERS")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| newest_capture("svg-rasters-"))
    else {
        eprintln!(
            "no Chromium SVG rasters: capture them (tools/svg-rasters) and set PM_SVG_RASTERS"
        );
        return;
    };
    let rasters = std::sync::Arc::new(RasterDir::new(&dir));
    let only = std::env::var("PM_SVG_ONLY").ok();
    let diffs = std::env::var("PM_SVG_DIFFS")
        .ok()
        .map(std::path::PathBuf::from);
    let mut renderer = Renderer::headless().expect("a Metal device");
    renderer.set_svg_wait(true);
    renderer.set_svg_source(Some(rasters.clone()));
    let (mut compared, mut projects, mut svg_pixels, mut invisible) =
        (0usize, 0usize, 0usize, 0usize);
    let mut failed = Vec::new();
    let mut skipped = Vec::new();
    for capture in &captures {
        let dpr = std::fs::read_to_string(capture.join("manifest.json"))
            .ok()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(&m).ok())
            .and_then(|m| m["gpu"]["devicePixelRatio"].as_f64())
            .unwrap_or(1.0) as f32;
        let mut measure =
            pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new());
        if dpr != 1.0 {
            measure.compat().set_font_cache_seeds(
                [12.0f32, 13.0]
                    .iter()
                    .map(|&css| pm_text::compat::FontCacheSeed {
                        family: "system-ui".into(),
                        weight: 400.0,
                        italic: false,
                        size: pm_text::compat::canvas::effective_font_size(css * dpr),
                        specified: css,
                    })
                    .collect(),
            );
        }
        renderer.set_text_fonts(Some(Box::new(text::MainThreadFonts(measure, None))));
        for pdir in projects_of(capture) {
            let p = load_project(capture, &pdir);
            if only.as_deref().is_some_and(|o| !p.name.starts_with(o)) {
                continue;
            }
            let Some(hidden) = without_svg_images(&p.document) else {
                continue;
            };
            projects += 1;
            if !p.missing_media.is_empty() {
                skipped.push((
                    p.name.clone(),
                    format!("media not available: {:?}", p.missing_media),
                ));
                continue;
            }
            // As the capture app: the built-in definitions, others missing.
            let mut set = all_builtin_definitions();
            set.missing = extension_definitions(&p.document)
                .into_iter()
                .filter(|id| !set.definitions.iter().any(|d| d.id.as_str() == id))
                .map(|id| id.as_str().into())
                .collect();
            renderer
                .set_definitions(&set)
                .expect("built-in definitions");
            // With fonts and definitions both: what each check lists only
            // for lack of the other is not a reason.
            let with_fonts =
                renderer.unsupported_with_media(&p.document, &p.composition, &p.resolver);
            let with_definitions = pm_render::unsupported_with_definitions(
                &p.document,
                &p.composition,
                Some(&p.resolver),
                renderer.effects(),
            );
            let issues: Vec<_> = with_fonts
                .into_iter()
                .filter(|u| with_definitions.contains(u))
                .collect();
            if !issues.is_empty() {
                let mut reasons: Vec<String> =
                    issues.iter().map(|u| u.feature.to_string()).collect();
                reasons.sort();
                reasons.dedup();
                skipped.push((p.name.clone(), reasons.join(", ")));
                continue;
            }
            let no_text = text::without_text(&p.document);
            for s in &p.samples {
                let mut request =
                    RenderRequest::sized(Size::new(s.width, s.height), Precision::Float16);
                request.motion_blur = pm_render::motion::MotionBlur::Samples(16);
                let render = |r: &mut Renderer, doc: &pm_model::Document| {
                    r.render(doc, &p.composition, ticks(s.time), &request, &p.resolver)
                        .map(|f| opaque8(&r.read_rgba(&f).unwrap()))
                };
                let ours = match render(&mut renderer, &p.document) {
                    Ok(o) => o,
                    Err(e) => {
                        failed.push(format!("{} t={}: render error {e}", p.name, s.time));
                        continue;
                    }
                };
                let base = render(&mut renderer, &hidden).expect("render without SVG images");
                let plain = render(&mut renderer, &no_text).expect("render without text");
                let (w, h, reference) = read_png(&s.png);
                assert_eq!((w, h), (s.width, s.height));
                let (wu, hu) = (w as usize, h as usize);
                let differs = |a: &[u8], b: &[u8]| -> Vec<bool> {
                    a.chunks_exact(4)
                        .zip(b.chunks_exact(4))
                        .map(|(x, y)| x != y)
                        .collect()
                };
                let mut text_mask = differs(&ours, &plain);
                for _ in 0..3 {
                    text_mask = dilate(&text_mask, wu, hu);
                }
                let svg_mask: Vec<bool> = differs(&ours, &base)
                    .iter()
                    .zip(&text_mask)
                    .map(|(&s, &t)| s && !t)
                    .collect();
                let svg = Diff::of_masked(&ours, &reference, &svg_mask);
                let rest_mask: Vec<bool> = svg_mask
                    .iter()
                    .zip(&text_mask)
                    .map(|(&s, &t)| !s && !t)
                    .collect();
                let rest = Diff::of_masked(&ours, &reference, &rest_mask);
                let map = ssim_map(&reference, &ours, wu, hu);
                let (mut sum, mut n) = (0.0, 0usize);
                for i in 0..wu * hu {
                    if text_mask[i] {
                        sum += map[i];
                        n += 1;
                    }
                }
                let text_ssim = if n > 0 { sum / n as f64 } else { 1.0 };
                compared += 1;
                svg_pixels += svg.pixels;
                let line = format!(
                    "{:<34} t={:<5} SVG px {:>7} exact {:>7} max {:>3} | text px {:>6} SSIM {:.4} | rest ≤1 {:>7.3}% max {}",
                    p.name,
                    s.time,
                    svg.pixels,
                    svg.exact,
                    svg.max,
                    n,
                    text_ssim,
                    100.0 * rest.fraction_within_one(),
                    rest.max
                );
                println!("{line}");
                if svg.pixels == 0 {
                    // The SVG images draw nothing visible at this time (a
                    // hidden or solo'd-out layer, covered content).
                    invisible += 1;
                }
                if svg.max != 0 {
                    failed.push(line);
                    if let Some(d) = &diffs {
                        std::fs::create_dir_all(d).unwrap();
                        let stem = format!("{}-{}", p.name.replace('/', "_"), s.time);
                        write_png(&d.join(format!("{stem}-engine.png")), w, h, &ours);
                        write_png(&d.join(format!("{stem}-legacy.png")), w, h, &reference);
                    }
                }
            }
        }
    }
    if !rasters.missing().is_empty() {
        let listed = rasters.write_needs().expect("needs.json");
        println!(
            "\n{} Chromium SVG rasters not captured ({listed} listed in {}/needs.json)",
            rasters.missing().len(),
            dir.display()
        );
    }
    println!("\nskipped:");
    for (name, why) in &skipped {
        println!("  {name:<34} {why}");
    }
    println!(
        "\n{projects} projects with level 1 SVG images; compared {compared} frames ({svg_pixels} SVG pixels; {invisible} frames show none); {} failed",
        failed.len()
    );
    for f in &failed {
        println!("  FAILED {f}");
    }
    assert!(
        failed.is_empty(),
        "{} frames whose SVG pixels differ",
        failed.len()
    );
}

/// The document with every level 1 SVG image layer at opacity 0, or `None`
/// when it has none.
#[cfg(target_os = "macos")]
fn without_svg_images(doc: &pm_model::Document) -> Option<pm_model::Document> {
    let mut root = doc.to_value()?;
    let svg_assets: Vec<String> = root
        .as_object()
        .and_then(|o| o.get("assets"))
        .and_then(|a| a.as_object())
        .map(|a| {
            a.iter()
                .filter(|(_, r)| {
                    r.as_object()
                        .and_then(|r| r.get("format"))
                        .and_then(|f| f.as_str())
                        == Some("svg")
                })
                .map(|(k, _)| k.to_string())
                .collect()
        })
        .unwrap_or_default();
    let mut any = false;
    let mut hide = |list: &mut Vec<pm_model::Value>| {
        for l in list.iter_mut() {
            let Some(o) = l.as_object_mut() else { continue };
            let image = o.get("type").and_then(|t| t.as_str()) == Some("image");
            let level1 = o
                .get("compat")
                .and_then(|c| c.as_f64())
                .is_none_or(|c| c < 2.0);
            let asset = o
                .get("d")
                .and_then(|d| d.as_object())
                .and_then(|d| d.get("asset"))
                .and_then(|a| a.as_str())
                .map(str::to_owned);
            if !(image && level1 && asset.is_some_and(|a| svg_assets.contains(&a))) {
                continue;
            }
            any = true;
            let Some(p) = o.get_mut("p").and_then(|p| p.as_object_mut()) else {
                continue;
            };
            let mut ch = pm_model::json::Map::new();
            ch.insert("v", pm_model::Value::Number(0.0));
            ch.insert("kf", pm_model::Value::Array(Vec::new()));
            ch.insert("expr", pm_model::Value::Null);
            p.insert("opacity", pm_model::Value::Object(ch));
        }
    };
    if let Some(o) = root.as_object_mut() {
        if let Some(l) = o.get_mut("layers").and_then(|l| l.as_array_mut()) {
            hide(l);
        }
        if let Some(c) = o.get_mut("comps").and_then(|c| c.as_object_mut()) {
            for (_, comp) in c.iter_mut() {
                if let Some(l) = comp
                    .as_object_mut()
                    .and_then(|c| c.get_mut("layers"))
                    .and_then(|l| l.as_array_mut())
                {
                    hide(l);
                }
            }
        }
    }
    any.then(|| pm_model::Document::new(Some(root), doc.revision() + 1))
}

#[cfg(target_os = "macos")]
fn projects_of(capture: &std::path::Path) -> Vec<std::path::PathBuf> {
    support::projects(capture)
}

#[cfg(not(target_os = "macos"))]
fn main() {}
