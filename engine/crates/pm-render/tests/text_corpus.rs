//! Text and captions in whole frames against the legacy renderer (plan
//! section 25, text class): every corpus project with text or captions
//! layers that this renderer supports is rendered at every captured sample
//! and compared with the capture's opaque frame.
//!
//! Classes: the *text pixels* are those our frame changes when its text and
//! captions layers are hidden (opacity 0), grown by 3 pixels. Every text
//! pixel is within 2 code values of the capture. SSIM over the text pixels
//! is reported. Other pixels use G-C2's independent authored f64 reference
//! and per-pixel closeness proof. Unreferenced content keeps the Standard
//! class (≤ 1 CV for 99.9%, ≤ 4 for any), with measured owned pending entries
//! for reference gaps. `tests/corpus.rs` leaves these projects to this test.
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
//! Level 2 text (`compat` ≥ 2, plan Appendix A) has no legacy reference:
//! today's engine ignores `compat` and draws it at level 1, and the capture
//! leaves it out (`engine/corpus/src/legacy-capture.ts`). A capture that still
//! holds such a project has it routed to the native semantic gate
//! (`tests/text_level2_semantics.rs`) after the routing check: counted,
//! listed, and a failure when the semantic gate does not judge it.
//!
//! `PM_ENGINE_CORPUS` picks captures (default: the newest `v1-*`, `render-*`
//! and `text-proj-*`, plus the newest `text-iso-*`); `PM_TEXT_ONLY` limits
//! project names by prefix; `PM_TEXT_DIFFS=<dir>` writes engine and legacy
//! PNGs of failures (every frame with `PM_TEXT_DIFFS_ALL`). A full run
//! (neither `PM_ENGINE_CORPUS` nor `PM_TEXT_ONLY`) on a machine with the
//! private corpus directory fails when any FONTS-R corpus entry was not
//! measured; without the directory the test reports and passes vacuously.
//!
//! Runs without the harness (the CoreText registry needs the main thread).

#[path = "support/pending.rs"]
mod pending;
#[path = "support/quality.rs"]
mod quality;
mod support;

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_protocol::common::Size;
    use pm_render::parity::{dilate, ssim_map};
    use pm_render::{RenderRequest, Renderer};
    use support::*;

    // Selecting a capture directory never waives completeness. Only the
    // explicit per-project developer filter makes this a narrowed run.
    let narrowed = std::env::var_os("PM_TEXT_ONLY").is_some();

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
    // The built-in effects, transitions, and extension layers as the kernel
    // snapshots them (as tests/corpus.rs sets them).
    renderer
        .set_definitions(&pm_render::all_builtin_definitions())
        .expect("built-in definitions");
    let mut compared = 0;
    let mut failed = Vec::new();
    let mut pending = pending::Audit::read(include_str!("fixtures/text-corpus-pending.json"));
    let mut skipped = Vec::new();
    // Full-frame text is allowed an owned reference gap only after the same
    // authored sample passes the unchanged text-only bar, or all differing
    // pixels are independently classified mask edges. Defer admission until
    // the isolation capture, which is measured last, has been read.
    let mut isolated_proofs = std::collections::BTreeMap::new();
    let mut context_observations = Vec::new();
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
            let authored_text =
                text::authored_isolation_fingerprint(&p.document).expect("authored text");
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
                // Path mask edges of 2D masked layers are corrected (exact
                // coverage and Gaussian): G-V2 (`tests/path_masks.rs`)
                // judges them against the reference, not this comparison.
                let bands = support::mask_reference::mask_bands(
                    &p.document,
                    &p.composition,
                    ticks(s.time),
                    wu,
                    hu,
                )
                .expect("mask reference");
                let rest: Vec<bool> = mask.iter().zip(&bands).map(|(m, b)| !m && !b).collect();
                let d = Diff::of_masked(&ours, &reference, &rest);
                // Text retains its exact legacy raster bar. Non-text pixels
                // use the same authored f64 proof as G-C2: corrected vector
                // coverage must be judged against its actual outline, not
                // against the old density-bucket raster in the capture.
                let r = support::reference::render(
                    &hidden,
                    &p.composition,
                    ticks(s.time),
                    w,
                    h,
                    true,
                    16,
                    renderer.effects(),
                    &p.resolver,
                );
                let opaque = support::reference::Reference {
                    rgba: r.opaque(),
                    ..r.clone()
                };
                let classes = opaque.classes();
                let selected = |values: &[f64]| {
                    values
                        .chunks_exact(4)
                        .zip(&rest)
                        .filter(|(_, keep)| **keep)
                        .flat_map(|(px, _)| px.iter().copied())
                        .collect::<Vec<_>>()
                };
                let engine = selected(
                    &ours
                        .iter()
                        .map(|v| f64::from(*v) / 255.0)
                        .collect::<Vec<_>>(),
                );
                let today = selected(
                    &reference
                        .iter()
                        .map(|v| f64::from(*v) / 255.0)
                        .collect::<Vec<_>>(),
                );
                let wanted = selected(&opaque.rgba);
                let classes: Vec<_> = classes
                    .into_iter()
                    .zip(&rest)
                    .filter_map(|(c, keep)| keep.then_some(c))
                    .collect();
                let proof = pm_reference::compare::compare(&engine, &today, &wanted, &classes);
                let problems = quality::reference_problems(
                    &proof,
                    &pm_reference::compare::Bounds::EIGHT_BIT,
                    &engine,
                    &today,
                    &wanted,
                    &classes,
                );
                let eligible = !r.unmodelled.is_empty()
                    && problems.iter().all(|p| p.starts_with("unreferenced:"));
                let owner = if r.unmodelled.iter().any(|reason| reason == "effects") {
                    "L6"
                } else {
                    "L4"
                };
                let owned = pending.check(
                    pending::Entry {
                        frame: pending::frame_id(
                            &format!("{}{tag}", p.name),
                            s.time,
                            w,
                            h,
                            "non-text",
                        ),
                        owner: owner.into(),
                        reason: r.unmodelled.join(", "),
                        checks: problems.clone(),
                        measurement_sha256: pending::measurement(
                            &engine, &today, &wanted, &classes,
                        ),
                    },
                    eligible,
                );
                let rest_ok = problems.is_empty() || owned;
                let pair = pending::frame_id(
                    &format!("{}:{authored_text}", p.name),
                    s.time,
                    w,
                    h,
                    "text-pair",
                );
                let engine_full: Vec<f64> = ours.iter().map(|v| f64::from(*v) / 255.0).collect();
                let today_full: Vec<f64> =
                    reference.iter().map(|v| f64::from(*v) / 255.0).collect();
                let signature = pending::measurement(
                    &engine_full,
                    &today_full,
                    &opaque.rgba,
                    &opaque.classes(),
                );
                if in_isolation && over2 == 0 {
                    isolated_proofs.insert(pair.clone(), signature.clone());
                }
                let outside_mask_band = (0..wu * hu)
                    .filter(|&i| mask[i] && !bands[i])
                    .filter(|&i| (0..4).any(|c| ours[i * 4 + c].abs_diff(reference[i * 4 + c]) > 2))
                    .count();
                compared += 1;
                covered.insert(p.name.clone());
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
                println!("{line}\n    non-text f64 reference{proof}");
                for problem in &problems {
                    println!("    {problem}");
                }
                context_observations.push((
                    pair, in_isolation, outside_mask_band,
                    pending::Entry {
                        frame: pending::frame_id(&format!("{}{tag}", p.name), s.time, w, h, "text-context"),
                        owner: owner.into(),
                        reason: r.unmodelled.join(", "),
                        checks: if over2 == 0 { Vec::new() } else {
                            vec![format!("text: {over2} pixels over 2 CV; max {tmax}; outside independent mask band {outside_mask_band}")]
                        },
                        measurement_sha256: signature,
                    }, line.clone(),
                ));
                let fails = over2 > 0 || !rest_ok;
                if !rest_ok {
                    failed.push(line);
                }
                // `PM_TEXT_DIFFS_ALL` writes the images of passing frames too
                // (side-by-sides for review).
                if (over2 > 0 || fails || std::env::var("PM_TEXT_DIFFS_ALL").is_ok())
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
    let mut text_pending = 0;
    for (pair, in_isolation, outside_mask_band, mut row, line) in context_observations {
        let isolated = isolated_proofs.get(&pair);
        let mask_edge_only = !row.checks.is_empty() && outside_mask_band == 0;
        let eligible = quality::text_context_is_owned_gap(
            in_isolation,
            isolated.is_some(),
            usize::from(!row.checks.is_empty()),
            outside_mask_band,
        );
        if mask_edge_only {
            row.owner = "L4".into();
            row.reason = "path mask/text edge composition; every differing text pixel is in the independent mask band".into();
        } else if let Some(proof) = isolated {
            row.reason = format!(
                "text composition with passing authored-isolation projection; {}",
                row.reason
            );
            if !row.checks.is_empty() {
                row.checks.push(format!(
                    "paired isolated text passes 2 CV; measurement {proof}"
                ));
            }
        }
        let bad = !row.checks.is_empty();
        let owned = pending.check(row, eligible);
        if bad && !owned {
            failed.push(line);
        }
        text_pending += usize::from(bad && owned);
    }
    println!("owned text-context pending: {text_pending}; original text bar remains 2 CV");
    failed.extend(pending.finish(!narrowed));
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
