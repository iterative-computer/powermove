//! Text and captions in whole frames against the legacy renderer (plan
//! section 25, text class): every corpus project with text or captions
//! layers that this renderer supports is rendered at every captured sample
//! and compared with the capture's opaque frame.
//!
//! Classes: the *text pixels* are those our frame changes when its text and
//! captions layers are hidden (opacity 0), grown by 3 pixels; they are
//! judged by SSIM (the mean of the four channels' maps) over those pixels,
//! at least 0.98. Every other pixel keeps the Standard class (≤ 1 CV for
//! 99.9%, ≤ 4 for any). `tests/corpus.rs` leaves these projects to this test.
//!
//! The capture's renderer laid `system-ui` DOM text out at 12 and 13 px on a
//! 2x display, which fixes `opsz` for 24 and 26 px `system-ui` canvas fonts
//! (amendment 19, `docs/engine/text.md`); the registry is seeded the same
//! way from the capture's device pixel ratio.
//!
//! Fonts come the way the engines and `pm-cli` give them: a served
//! registry (`pm_text::served`) over full fonts, with the capture renderer's
//! seeds, and level 1 glyphs from CoreGraphics masks. Routing is forbidden
//! for text: a project the renderer refuses for a text reason fails the
//! test. Projects refused for features outside text (effects without a
//! definition, SVG images, masks, motion blur) are listed with their
//! reasons and left to `tests/corpus.rs`.
//!
//! `PM_ENGINE_CORPUS` picks captures (default: the newest `v1-*`, `render-*`
//! and `text-proj-*`); `PM_TEXT_ONLY` limits project names by prefix;
//! `PM_TEXT_DIFFS=<dir>` writes engine and legacy PNGs of failures.
//!
//! Runs without the harness (the CoreText registry needs the main thread).

mod support;

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_protocol::common::Size;
    use pm_render::parity::{dilate, ssim_map};
    use pm_render::{RenderRequest, Renderer};
    use support::*;

    let mut captures = corpus_dirs();
    if std::env::var("PM_ENGINE_CORPUS").is_err()
        && let Some(t) = newest_capture("text-proj-")
    {
        captures.push(t);
    }
    if captures.is_empty() {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let only = std::env::var("PM_TEXT_ONLY").ok();
    let diffs = std::env::var("PM_TEXT_DIFFS")
        .ok()
        .map(std::path::PathBuf::from);
    let mut renderer = Renderer::headless().expect("a Metal device");
    let mut compared = 0;
    let mut failed = Vec::new();
    let mut skipped = Vec::new();
    for capture in &captures {
        let dpr = std::fs::read_to_string(capture.join("manifest.json"))
            .ok()
            .and_then(|m| serde_json::from_str::<serde_json::Value>(&m).ok())
            .and_then(|m| m["gpu"]["devicePixelRatio"].as_f64())
            .unwrap_or(1.0) as f32;
        // The registry served on this thread, as `pm-cli` serves it; the
        // engines serve it from their main thread to render threads.
        let server = std::sync::Arc::new(pm_text::served::DirectServer::new(
            pm_text::registry::CoreTextRegistry::new(),
            |r| r.generation(),
        ));
        let seeds = pm_text::compat::app_ui_font_cache_seeds(&[dpr]);
        renderer.set_text_fonts(Some(Box::new(pm_render::text::served::ServedFonts::new(
            server,
            std::sync::Arc::new(pm_render::text::served::FixedSeeds(seeds)),
        ))));
        for dir in projects(capture) {
            let p = load_project(capture, &dir);
            if only.as_deref().is_some_and(|o| !p.name.starts_with(o)) {
                continue;
            }
            if !text::has_text(&p.document) {
                continue;
            }
            if !p.missing_media.is_empty() {
                skipped.push((
                    p.name.clone(),
                    format!("media not available: {:?}", p.missing_media),
                ));
                continue;
            }
            // The capture app had the built-in definitions only.
            let mut set = pm_render::all_builtin_definitions();
            set.missing = support::extension_definitions(&p.document)
                .into_iter()
                .filter(|id| !set.definitions.iter().any(|d| d.id.as_str() == id))
                .map(|id| id.as_str().into())
                .collect();
            renderer.set_definitions(&set).expect("built-in definitions");
            let with_fonts =
                renderer.unsupported_with_media(&p.document, &p.composition, &p.resolver);
            let issues: Vec<_> = pm_render::unsupported_with_definitions(
                &p.document,
                &p.composition,
                Some(&p.resolver),
                renderer.effects(),
            )
            .into_iter()
            // Drawn: the renderer has fonts (the static check without them
            // lists every text layer).
            .filter(|u| !matches!(&u.feature, pm_render::Feature::Text(_)))
            .chain(
                with_fonts
                    .into_iter()
                    .filter(|u| matches!(&u.feature, pm_render::Feature::Text(_))),
            )
            .collect();
            let text_refusals: Vec<String> = issues
                .iter()
                .filter(|u| matches!(&u.feature, pm_render::Feature::Text(_)))
                .map(ToString::to_string)
                .collect();
            if !text_refusals.is_empty() {
                failed.push(format!(
                    "{}: text refused (routing is forbidden): {}",
                    p.name,
                    text_refusals.join("; ")
                ));
                continue;
            }
            if !issues.is_empty() {
                let mut reasons: Vec<String> =
                    issues.iter().map(|u| u.feature.to_string()).collect();
                reasons.sort();
                reasons.dedup();
                skipped.push((p.name.clone(), reasons.join(", ")));
                continue;
            }
            let hidden = text::without_text(&p.document);
            for s in &p.samples {
                let request =
                    RenderRequest::sized(Size::new(s.width, s.height), Precision::Float16);
                let render = |r: &mut Renderer, doc: &pm_model::Document| {
                    r.render(doc, &p.composition, ticks(s.time), &request, &p.resolver)
                        .map(|f| opaque8(&r.read_rgba(&f).unwrap()))
                };
                let ours = match render(&mut renderer, &p.document) {
                    Ok(o) => o,
                    Err(pm_render::RenderError::Unsupported(u))
                        if u.iter().all(|u| !matches!(&u.feature, pm_render::Feature::Text(_))) =>
                    {
                        skipped.push((
                            format!("{} t={}", p.name, s.time),
                            u.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "),
                        ));
                        continue;
                    }
                    Err(e) => {
                        failed.push(format!("{} t={}: render error {e}", p.name, s.time));
                        continue;
                    }
                };
                let base = render(&mut renderer, &hidden).expect("render without text");
                let (w, h, reference) = read_png(&s.png);
                assert_eq!((w, h), (s.width, s.height));
                let (wu, hu) = (w as usize, h as usize);
                let mut mask: Vec<bool> = ours
                    .chunks_exact(4)
                    .zip(base.chunks_exact(4))
                    .map(|(a, b)| a != b)
                    .collect();
                for _ in 0..3 {
                    mask = dilate(&mask, wu, hu);
                }
                let map = ssim_map(&reference, &ours, wu, hu);
                let (mut sum, mut n) = (0.0, 0usize);
                for i in 0..wu * hu {
                    if mask[i] {
                        sum += map[i];
                        n += 1;
                    }
                }
                let text_ssim = if n > 0 { sum / n as f64 } else { 1.0 };
                let rest: Vec<bool> = mask.iter().map(|m| !m).collect();
                let d = Diff::of_masked(&ours, &reference, &rest);
                compared += 1;
                let line = format!(
                    "{:<36} t={:<6} text px {:>7} SSIM {:.4} | rest ≤1 {:>7.3}% max {}",
                    p.name,
                    s.time,
                    n,
                    text_ssim,
                    100.0 * d.fraction_within_one(),
                    d.max
                );
                println!("{line}");
                if text_ssim < 0.98 || !d.passes() {
                    failed.push(line);
                    if let Some(dir) = &diffs {
                        std::fs::create_dir_all(dir).unwrap();
                        let stem = format!("{}-{}", p.name.replace('/', "_"), s.time);
                        write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                        write_png(&dir.join(format!("{stem}-legacy.png")), w, h, &reference);
                    }
                }
            }
        }
    }
    println!("\nnot compared (features outside text; tests/corpus.rs):");
    for (name, why) in &skipped {
        println!("  {name:<36} {why}");
    }
    println!("\ncompared {compared} frames; {} failed", failed.len());
    for f in &failed {
        println!("  FAILED {f}");
    }
    assert!(
        failed.is_empty(),
        "{} frames outside the text class",
        failed.len()
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
