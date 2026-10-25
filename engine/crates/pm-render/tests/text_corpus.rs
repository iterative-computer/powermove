//! G-T2: text and captions in whole frames against independent f64 outline
//! coverage, float paint and frame composition. Every supported corpus
//! project is rendered at each captured sample. The capture remains today's
//! quality witness; it is never the modeled reference.
//!
//! The plan's device-space text scenes provide only exact layout and authored
//! drawing inputs. `support::text_reference` scans and composites their
//! outlines independently. Text pixels use the independently modeled
//! composition background and, when paint order permits, modeled solids.
//! Other nontext content must not overlap them.
//! Unsupported text composition remains an explicit failing reference gap,
//! while pixels outside text retain the original Standard bound.
//!
//! The capture's renderer laid `system-ui` DOM text out at 12 and 13 px on a
//! 2x display, which fixes `opsz` for 24 and 26 px `system-ui` canvas fonts
//! (amendment 19, `docs/engine/text.md`); the registry is seeded the same
//! way from the capture's device pixel ratio.
//!
//! Fonts come the way the engines and `pm-cli` give them: a served
//! registry (`pm_text::served`) over full fonts, with the capture renderer's
//! seeds and level 1 layout. Routing is forbidden
//! for text: a project the renderer refuses for a text reason fails the
//! test. A project refused for features outside text (SVG images, motion
//! blur, 2.5D, path masks, which other lanes draw) is compared through its
//! text in isolation: the newest `text-iso-*` capture holds every corpus
//! project with text reduced to its text (`engine/corpus/src/text-only.ts`,
//! `capture.ts --text-only`). A project compared neither way fails the test:
//! no text escapes the comparison.
//!
//! Level 2 text (`compat` ≥ 2, plan Appendix A) has no legacy reference:
//! today's engine ignores `compat` and draws it at level 1, and the capture
//! leaves it out (`engine/corpus/src/legacy-capture.ts`). A capture that still
//! holds such a project has it routed to the native semantic gate
//! (`tests/text_level2_semantics.rs`) after the routing check: counted,
//! listed, and a failure when the semantic gate does not judge it.
//!
//! `PM_ENGINE_CORPUS` picks captures (default: the newest `v1-*`, `render-*`
//! and `text-proj-*`, plus the newest `text-iso-*`); `PM_TEXT_ONLY` limits
//! project names by prefix; `PM_TEXT_DIFFS=<dir>` writes engine and witness
//! PNGs of failures (every frame with `PM_TEXT_DIFFS_ALL`). `PM_TEXT_REPORT`
//! and `PM_TEXT_DUMP` write external numeric reference verdicts and native
//! pixels for G-T5. A full run
//! (neither `PM_ENGINE_CORPUS` nor `PM_TEXT_ONLY`) on a machine with the
//! private corpus directory fails when any FONTS-R corpus entry was not
//! measured; without the directory the test reports and passes vacuously.
//!
//! Runs without the harness (the CoreText registry needs the main thread).

mod support;

#[cfg(target_os = "macos")]
fn stats_json(s: &pm_reference::compare::Stats) -> serde_json::Value {
    serde_json::json!({ "max": s.max, "p999": s.p999, "mean": s.mean, "pixels": s.pixels })
}

#[cfg(target_os = "macos")]
fn outlined_frame(
    renderer: &mut pm_render::Renderer,
    doc: &pm_model::Document,
    hidden: &pm_model::Document,
    comp_id: &str,
    time: f64,
    size: (u32, u32),
    resolver: &pm_render::MapResolver,
    hidden_pixels: &[u8],
) -> Result<(Vec<f64>, Vec<pm_reference::compare::Class>, Vec<bool>), String> {
    use pm_protocol::{color::Precision, common::Size};
    use pm_reference::compare::{Class, dilate};
    use pm_reference::composite::blend;

    let (w, h) = (size.0 as usize, size.1 as usize);
    if hidden_pixels.len() != w * h * 4 {
        return Err("hidden frame size".into());
    }
    let mut request =
        pm_render::RenderRequest::sized(Size::new(size.0, size.1), Precision::Float16);
    request.motion_blur = pm_render::motion::MotionBlur::Samples(16);
    let (plan, failures) = renderer
        .plan(doc, comp_id, support::ticks(time), &request, resolver)
        .map_err(|e| format!("{e}"))?;
    if !failures.is_empty() {
        return Err(format!("frame plan: {failures:?}"));
    }
    // A completed hidden frame is a valid underlay only if every nontext
    // draw precedes every text draw in the plan's bottom-to-top paint order.
    // Otherwise a nontext draw above text would be composed in the wrong
    // order. Keep unsupported sources explicit through the strict fallback.
    let mut seen_text = false;
    let mut nontext_above_text = false;
    let mut text_draws = Vec::with_capacity(plan.draws.len());
    for draw in &plan.draws {
        let ideal = support::text_group_reference::render_draw(draw, size.0, size.1)?;
        if ideal.is_some() {
            seen_text = true;
        } else if seen_text {
            nontext_above_text = true;
        }
        text_draws.push(ideal);
    }
    let full_hidden = (seen_text && !nontext_above_text).then(|| {
        support::reference::render(hidden, comp_id, support::ticks(time), size.0, size.1, true)
    });
    let background = support::text::background_only(hidden);
    let mut base = match full_hidden {
        Some(full) if full.unmodelled.is_empty() && full.modelled() == 1.0 => full,
        _ => support::reference::render(
            &background,
            comp_id,
            support::ticks(time),
            size.0,
            size.1,
            true,
        ),
    };
    if !base.unmodelled.is_empty() {
        return Err(base.unmodelled.join("; "));
    }
    let background_pixels = base.opaque();
    let mut classes = vec![Class::Unreferenced; w * h];
    let mut text_seen = false;
    let mut text_reached = vec![false; w * h];
    let mut unknown_text = vec![false; w * h];
    for (draw, ideal) in plan.draws.iter().zip(text_draws) {
        let Some(ideal) = ideal else {
            continue;
        };
        text_seen = true;
        let mode = support::text_group_reference::blend_mode(draw);
        for i in 0..w * h {
            let s = &ideal.rgba[i * 4..][..4];
            let d = &mut base.rgba[i * 4..][..4];
            if s[3] > 0.0 && ideal.classes[i] != Class::Unreferenced {
                let background_delta = (0..4)
                    .map(|c| {
                        (f64::from(hidden_pixels[i * 4 + c]) / 255.0 - background_pixels[i * 4 + c])
                            .abs()
                            * 255.0
                    })
                    .fold(0.0f64, f64::max);
                if background_delta > 1.0 + 1e-9 {
                    return Err(format!(
                        "nontext content intersects text at pixel {i}: background delta {background_delta:.2} CV"
                    ));
                }
            }
            // An unmodeled float image remains an explicit original-bound
            // gap. Mark its reach even
            // when the reference stored NaN, so the gap is actually checked.
            if ideal.classes[i] == Class::Unreferenced {
                unknown_text[i] = true;
                classes[i] = Class::Unreferenced;
                text_reached[i] = true;
                continue;
            }
            if s[3] <= 0.0 {
                continue;
            }
            let result = blend(mode, [s[0], s[1], s[2], s[3]], [d[0], d[1], d[2], d[3]]);
            d.copy_from_slice(&result);
            if s[3] > 0.0 {
                text_reached[i] = true;
            }
            classes[i] = if unknown_text[i] {
                Class::Unreferenced
            } else {
                match (classes[i], ideal.classes[i]) {
                    (Class::Resampled, _) | (_, Class::Resampled) => Class::Resampled,
                    (Class::Edge, _) | (_, Class::Edge) => Class::Edge,
                    (_, next) => next,
                }
            };
        }
    }
    if !text_seen {
        return Err("no active text".into());
    }
    let reach = dilate(&text_reached, w, h, 2);
    for i in 0..w * h {
        if !reach[i] || classes[i] != Class::Unreferenced || unknown_text[i] {
            continue;
        }
        let background_delta = (0..4)
            .map(|c| {
                (f64::from(hidden_pixels[i * 4 + c]) / 255.0 - background_pixels[i * 4 + c]).abs()
                    * 255.0
            })
            .fold(0.0f64, f64::max);
        if background_delta > 1.0 + 1e-9 {
            return Err(format!(
                "nontext content intersects text halo at pixel {i}: background delta {background_delta:.2} CV"
            ));
        }
        // The reference has only independently modeled background here;
        // any engine ink in this near-outline halo is a real edge error.
        classes[i] = Class::Edge;
    }
    Ok((base.opaque(), classes, reach))
}

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_protocol::common::Size;
    use pm_render::parity::dilate;
    use pm_render::{RenderRequest, Renderer};
    use sha2::{Digest, Sha256};
    use support::*;

    let narrowed = std::env::var_os("PM_ENGINE_CORPUS").is_some()
        || std::env::var_os("PM_TEXT_ONLY").is_some();

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
    let mut level2 = text::Level2Routes::new();
    if captures.is_empty() && (narrowed || corpus_root().is_none()) {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let only = std::env::var("PM_TEXT_ONLY").ok();
    let diffs = std::env::var("PM_TEXT_DIFFS")
        .ok()
        .map(std::path::PathBuf::from);
    let mut renderer = Renderer::headless().expect("a Metal device");
    // Level 1 SVG images (decision S1) draw Chromium's rasters, as
    // tests/corpus.rs draws them: PM_SVG_RASTERS, else the newest
    // `svg-rasters-*` directory; without one resvg approximates them.
    renderer.set_svg_wait(true);
    match std::env::var("PM_SVG_RASTERS")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| newest_capture("svg-rasters-"))
    {
        Some(dir) => renderer.set_svg_source(Some(std::sync::Arc::new(
            pm_render::vector::svg_rasters::RasterDir::new(dir),
        ))),
        None => {
            eprintln!(
                "no Chromium SVG rasters (PM_SVG_RASTERS): resvg approximates level 1 SVG images"
            );
            renderer.set_svg_rasterizer(pm_render::vector::svg_rasters::SvgRasterizer::Resvg);
        }
    }
    // The built-in effects, transitions, and extension layers as the kernel
    // snapshots them (as tests/corpus.rs sets them).
    renderer
        .set_definitions(&pm_render::all_builtin_definitions())
        .expect("built-in definitions");
    let mut compared = 0;
    let mut failed = Vec::new();
    let mut gate_rows = Vec::new();
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
            // Level 2 text has no legacy reference (today's engine draws it
            // at level 1): it is judged by the semantic gate, after the
            // routing check below, and never against this capture.
            let level2 = level2.route(&format!("{}{tag}", p.name), &p.name, &p.document);
            if !level2 && !p.missing_media.is_empty() {
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
            if level2 {
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
            for (sample_index, s) in p.samples.iter().enumerate() {
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
                    .zip(reference.chunks_exact(4))
                    .map(|((a, b), today)| a != b || today != b)
                    .collect();
                for _ in 0..3 {
                    mask = dilate(&mask, wu, hu);
                }
                // The independent reference models exact outline coverage,
                // float paint, captions and image glyph sampling where the
                // flat document has no intervening feature. Unsupported
                // text composition remains a failing reference gap.
                let ideal = outlined_frame(
                    &mut renderer,
                    &p.document,
                    &hidden,
                    &p.composition,
                    s.time,
                    (w, h),
                    &p.resolver,
                    &base,
                );
                let mut why = Vec::new();
                let mut gap = None;
                let mut modeled_reach = None;
                let mut unreferenced_text = 0usize;
                let mut reference_metrics = serde_json::Value::Null;
                match ideal {
                    Ok((rgba, classes, reached)) => {
                        use pm_reference::compare::{Bounds, Class, compare};
                        let e: Vec<f64> = ours.iter().map(|x| f64::from(*x) / 255.0).collect();
                        let t: Vec<f64> = reference.iter().map(|x| f64::from(*x) / 255.0).collect();
                        let report = compare(&e, &t, &rgba, &classes);
                        // Modeled text is held to one CV at each pixel. The
                        // original Standard rule still judges unmodeled
                        // nontext through `rest_ok` below.
                        why.extend(
                            report
                                .failures(&Bounds {
                                    per_pixel_cv: (1.0, 1.0),
                                    ..Bounds::EIGHT_BIT
                                })
                                .into_iter()
                                .filter(|s| !s.starts_with("unreferenced:")),
                        );
                        unreferenced_text = classes
                            .iter()
                            .zip(&reached)
                            .filter(|(class, reached)| **class == Class::Unreferenced && **reached)
                            .count();
                        for (i, class) in classes.iter().enumerate() {
                            if *class != Class::Unreferenced || !reached[i] {
                                continue;
                            }
                            let d = (0..4)
                                .map(|c| ours[i * 4 + c].abs_diff(reference[i * 4 + c]))
                                .max()
                                .unwrap();
                            if d > 2 {
                                why.push(format!(
                                    "unreferenced text pixel {i}: {d} CV > original 2"
                                ));
                                break;
                            }
                        }
                        let edge = report.class(Class::Edge);
                        let image = report.class(Class::Resampled);
                        reference_metrics = serde_json::json!({
                            "edge": { "engine": stats_json(&edge.engine_vs_reference),
                                "today": stats_json(&edge.today_vs_reference),
                                "perPixelExcess": stats_json(&edge.excess) },
                            "image": { "engine": stats_json(&image.engine_vs_reference),
                                "today": stats_json(&image.today_vs_reference),
                                "perPixelExcess": stats_json(&image.excess) },
                            "interiorToday": stats_json(&report.class(Class::Interior).engine_vs_today),
                            "unreferencedToday": stats_json(&report.class(Class::Unreferenced).engine_vs_today),
                        });
                        println!(
                            "frame {}: edge engine {} / today {}; image engine {} / today {}",
                            compared,
                            edge.engine_vs_reference,
                            edge.today_vs_reference,
                            image.engine_vs_reference,
                            image.today_vs_reference
                        );
                        modeled_reach = Some(reached);
                    }
                    Err(reason) => gap = Some(reason),
                }
                let (mut over2, mut tmax, mut n) = (0usize, 0u8, 0usize);
                for i in 0..wu * hu {
                    if mask[i] {
                        n += 1;
                        let e = (0..4)
                            .map(|c| ours[i * 4 + c].abs_diff(reference[i * 4 + c]))
                            .max()
                            .unwrap();
                        tmax = tmax.max(e);
                        over2 += usize::from(e > 2);
                    }
                }
                // With a modeled scene, the independent ink footprint—not
                // differences from the hidden engine frame—defines text.
                // Thus unrelated nontext differences cannot disappear into
                // the text region of the old comparison.
                let rest: Vec<bool> = modeled_reach
                    .as_ref()
                    .unwrap_or(&mask)
                    .iter()
                    .map(|m| !m)
                    .collect();
                let d = Diff::of_masked(&ours, &reference, &rest);
                // A frame whose every pixel is text has no other pixels to
                // judge (the Standard class holds trivially).
                let rest_ok = !rest.contains(&true) || d.passes();
                compared += 1;
                let fails = gap.is_some() || !why.is_empty() || !rest_ok;
                exact += usize::from(!fails && gap.is_none());
                let frame_index = compared - 1;
                let engine_sha256 = format!("{:x}", Sha256::digest(&ours));
                gate_rows.push(serde_json::json!({
                    "case": frame_index, "pass": !fails && gap.is_none(),
                    "width": w, "height": h,
                    "captureDir": capture.display().to_string(),
                    "projectDir": dir.strip_prefix(capture).unwrap().display().to_string(),
                    "sampleIndex": sample_index, "time": s.time,
                    "engineSha256": engine_sha256,
                    "referenceModel": if gap.is_some() { "original-bound-gap" }
                        else if unreferenced_text > 0 { "outline-with-original-bound-gap" }
                        else { "outline" },
                    "unreferencedTextPixels": unreferenced_text,
                    "gapReason": gap.as_deref(),
                    "referenceMetrics": reference_metrics,
                    "originalBound": { "textPixels": n, "textOverTwo": over2,
                        "textMaxCv": tmax, "restMaxCv": d.max,
                        "restWithinOne": d.fraction_within_one() },
                }));
                if let Ok(dir) = std::env::var("PM_TEXT_DUMP") {
                    let dir = std::path::Path::new(&dir);
                    assert!(
                        !dir.starts_with(repo_root()),
                        "text evidence must stay outside Git"
                    );
                    std::fs::create_dir_all(dir).unwrap();
                    std::fs::write(dir.join(format!("frame-{frame_index}-engine.rgba")), &ours)
                        .unwrap();
                }
                let line = format!(
                    "frame {:>4}: text px {:>7} >2 CV {:>6} max {:>3} | rest ≤1 {:>7.3}% max {} | {}",
                    compared - 1,
                    n,
                    over2,
                    tmax,
                    100.0 * d.fraction_within_one(),
                    d.max,
                    gap.as_deref()
                        .map_or("exact outline reference", |_| "original-bound gap")
                );
                println!("{line}");
                if fails {
                    if !why.is_empty() {
                        println!("  frame {} why: {}", compared - 1, why.join("; "));
                    }
                    failed.push(format!("{line}: {}", why.join("; ")));
                }
                if !fails && gap.is_none() {
                    covered.insert(p.name.clone());
                }
                if let Some(reason) = gap {
                    println!("  frame {} gap: {reason}", compared - 1);
                }
                // `PM_TEXT_DIFFS_ALL` writes the images of passing frames too
                // (side-by-sides for review).
                if (over2 > 0 || fails || std::env::var("PM_TEXT_DIFFS_ALL").is_ok())
                    && let Some(dir) = &diffs
                {
                    std::fs::create_dir_all(dir).unwrap();
                    let stem = format!("frame-{}", compared - 1);
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
    if let Ok(path) = std::env::var("PM_TEXT_REPORT") {
        let path = std::path::Path::new(&path);
        assert!(
            !path.starts_with(repo_root()),
            "text evidence must stay outside Git"
        );
        std::fs::write(path, serde_json::to_vec_pretty(&gate_rows).unwrap()).unwrap();
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
    level2.report();
    failed.extend(level2.failures());
    println!(
        "\ncompared {compared} frames ({exact} exact within 2 code values); {} failed",
        failed.len()
    );
    for f in &failed {
        println!("  FAILED {f}");
    }
    assert!(
        failed.is_empty(),
        "{} frames violate the text class",
        failed.len()
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
