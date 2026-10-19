//! Text and captions in whole frames against the legacy renderer (plan
//! section 25, text class): every corpus project with text or captions
//! layers that this renderer supports is rendered at every captured sample
//! and compared with the capture's opaque frame.
//!
//! Classes: the *text pixels* are those our frame changes when its text and
//! captions layers are hidden (opacity 0), grown by 3 pixels. Text matches
//! today exactly (amendment 22): every text pixel within 2 code values of
//! the capture. Every difference beyond that bound fails. SSIM over the text
//! pixels is reported. Every other pixel keeps the Standard
//! class (≤ 1 CV for 99.9%, ≤ 4 for any). `tests/corpus.rs` leaves these
//! projects to this test.
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
//! test. A project refused for features outside text (SVG images, motion
//! blur, 2.5D, path masks, which other lanes draw) is compared through its
//! text in isolation: the newest `text-iso-*` capture holds every corpus
//! project with text reduced to its text (`engine/corpus/src/text-only.ts`,
//! `capture.ts --text-only`). A project compared neither way fails the test:
//! no text escapes the comparison.
//!
//! `PM_ENGINE_CORPUS` picks captures (default: the newest `v1-*`, `render-*`
//! and `text-proj-*`, plus the newest `text-iso-*`); `PM_TEXT_ONLY` limits
//! project names by prefix; `PM_TEXT_DIFFS=<dir>` writes engine and legacy
//! PNGs of failures (every frame with `PM_TEXT_DIFFS_ALL`).
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
    // Text in isolation, for projects refused for features outside text.
    let isolated = newest_capture("text-iso-");
    if let Some(iso) = &isolated
        && !captures.contains(iso)
    {
        captures.push(iso.clone());
    }
    // Projects compared in full or in isolation, by name.
    let mut covered = std::collections::BTreeSet::new();
    let mut exact = 0;
    if captures.is_empty() {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let only = std::env::var("PM_TEXT_ONLY").ok();
    let diffs = std::env::var("PM_TEXT_DIFFS")
        .ok()
        .map(std::path::PathBuf::from);
    let mut renderer = Renderer::headless().expect("a Metal device");
    // The built-in effects, transitions, and extension layers as the kernel
    // snapshots them (as tests/corpus.rs sets them).
    renderer
        .set_definitions(&pm_render::all_builtin_definitions())
        .expect("built-in definitions");
    let mut compared = 0;
    let mut failed = Vec::new();
    let mut skipped = Vec::new();
    for capture in &captures {
        let in_isolation = isolated.as_ref() == Some(capture);
        let tag = if in_isolation {
            " (text in isolation)"
        } else {
            ""
        };
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
                    format!("{}{tag}", p.name),
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
            renderer
                .set_definitions(&set)
                .expect("built-in definitions");
            let issues = pm_render::capability::unsupported_with_definitions_and_fonts(
                &p.document,
                &p.composition,
                Some(&p.resolver),
                renderer.effects(),
            );
            // Text routing is forbidden. Preserve integration's other
            // forbidden classes too; isolation cannot hide those routes.
            let forbidden: Vec<String> = issues
                .iter()
                .filter(|u| {
                    matches!(&u.feature, pm_render::Feature::Text(_))
                        || routing_forbidden(&u.feature)
                })
                .map(ToString::to_string)
                .collect();
            if !forbidden.is_empty() {
                failed.push(format!(
                    "{}{tag}: routed for {}",
                    p.name,
                    forbidden.join(", ")
                ));
                continue;
            }
            if !issues.is_empty() {
                let mut reasons: Vec<String> =
                    issues.iter().map(|u| u.feature.to_string()).collect();
                reasons.sort();
                reasons.dedup();
                skipped.push((
                    format!("{}{tag}", p.name),
                    p.name.clone(),
                    reasons.join(", "),
                ));
                continue;
            }
            let hidden = text::without_text(&p.document);
            for s in &p.samples {
                // The capture's motion blur: 16 samples (opaque export).
                let mut request =
                    RenderRequest::sized(Size::new(s.width, s.height), Precision::Float16);
                request.motion_blur = pm_render::motion::MotionBlur::Samples(16);
                let render = |r: &mut Renderer, doc: &pm_model::Document| {
                    r.render(doc, &p.composition, ticks(s.time), &request, &p.resolver)
                        .map(|f| opaque8(&r.read_rgba(&f).unwrap()))
                };
                let ours = match render(&mut renderer, &p.document) {
                    Ok(o) => o,
                    Err(pm_render::RenderError::Unsupported(u))
                        if u.iter().all(|u| {
                            !matches!(&u.feature, pm_render::Feature::Text(_))
                                && !routing_forbidden(&u.feature)
                        }) =>
                    {
                        skipped.push((
                            format!("{}{tag} t={}", p.name, s.time),
                            p.name.clone(),
                            u.iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(", "),
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
                // Exactness over the text pixels.
                let (mut over2, mut tmax) = (0usize, 0u8);
                for i in 0..wu * hu {
                    if mask[i] {
                        let e = (0..4)
                            .map(|c| ours[i * 4 + c].abs_diff(reference[i * 4 + c]))
                            .max()
                            .unwrap();
                        tmax = tmax.max(e);
                        over2 += usize::from(e > 2);
                    }
                }
                let rest: Vec<bool> = mask.iter().map(|m| !m).collect();
                let d = Diff::of_masked(&ours, &reference, &rest);
                // A frame whose every pixel is text has no other pixels to
                // judge (the Standard class holds trivially).
                let rest_ok = !rest.contains(&true) || d.passes();
                compared += 1;
                covered.insert(p.name.clone());
                let text_ok = over2 == 0;
                exact += usize::from(over2 == 0);
                let line = format!(
                    "{:<44} t={:<6} text px {:>7} SSIM {:.4} >2 CV {:>6} max {:>3} | rest ≤1 {:>7.3}% max {}",
                    format!("{}{tag}", p.name),
                    s.time,
                    n,
                    text_ssim,
                    over2,
                    tmax,
                    100.0 * d.fraction_within_one(),
                    d.max
                );
                println!("{line}");
                let fails = !text_ok || !rest_ok;
                if fails {
                    failed.push(line);
                }
                // `PM_TEXT_DIFFS_ALL` writes the images of passing frames too
                // (side-by-sides for review).
                if (fails || std::env::var("PM_TEXT_DIFFS_ALL").is_ok())
                    && let Some(dir) = &diffs
                {
                    std::fs::create_dir_all(dir).unwrap();
                    let stem = format!(
                        "{}{}-{}",
                        p.name.replace('/', "_"),
                        if in_isolation { "-text-only" } else { "" },
                        s.time
                    );
                    write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                    write_png(&dir.join(format!("{stem}-legacy.png")), w, h, &reference);
                    // Pixels the "rest" check compares that differ by
                    // more than 1 code value, magnified in red.
                    let mut rest_diff = vec![0u8; wu * hu * 4];
                    for i in 0..wu * hu {
                        let d = (0..3)
                            .map(|c| ours[i * 4 + c].abs_diff(reference[i * 4 + c]))
                            .max()
                            .unwrap_or(0);
                        rest_diff[i * 4 + 3] = 255;
                        if rest[i] && d > 1 {
                            rest_diff[i * 4] = d.saturating_mul(5).max(64);
                        } else if mask[i] {
                            rest_diff[i * 4 + 2] = 48;
                        }
                    }
                    write_png(&dir.join(format!("{stem}-rest.png")), w, h, &rest_diff);
                }
            }
        }
    }
    // Every project refused for features outside text is compared through
    // its text in isolation; one that is not is a failure (no escapes).
    println!("\nrefused for features outside text (compared in isolation instead):");
    for (label, name, why) in &skipped {
        let ok = covered.contains(name);
        println!(
            "  {label:<44} {why}{}",
            if ok { "" } else { "  NOT COMPARED" }
        );
        if !ok {
            failed.push(format!(
                "{label}: not compared ({why}) and no text-in-isolation capture covers it"
            ));
        }
    }
    println!(
        "\ncompared {compared} frames ({exact} exact within 2 code values); {} failed",
        failed.len()
    );
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
