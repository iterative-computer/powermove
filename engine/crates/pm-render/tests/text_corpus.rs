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
//! Text composition the reference does not model is a reference gap: it
//! fails unless it is a measured, owned pending entry. Pixels outside text
//! use G-C2's independent authored f64 reference and per-pixel closeness
//! proof; unreferenced non-text content keeps the Standard class (≤ 1 CV for
//! 99.9%, ≤ 4 for any) or is a measured owned pending entry
//! (`fixtures/text-corpus-pending.json`).
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

#[path = "support/pending.rs"]
mod pending;
#[path = "support/quality.rs"]
mod quality;
mod support;

#[cfg(target_os = "macos")]
fn stats_json(s: &pm_reference::compare::Stats) -> serde_json::Value {
    serde_json::json!({ "max": s.max, "p999": s.p999, "mean": s.mean, "pixels": s.pixels })
}

/// The modeled reference of one frame: opaque pixels, their classes, and
/// the text's dilated reach.
#[cfg(target_os = "macos")]
struct Outlined {
    rgba: Vec<f64>,
    classes: Vec<pm_reference::compare::Class>,
    reach: Vec<bool>,
}

#[cfg(target_os = "macos")]
fn outlined_frame(
    renderer: &mut pm_render::Renderer,
    project: &support::Project,
    hidden: &pm_model::Document,
    time: f64,
    size: (u32, u32),
    hidden_pixels: &[u8],
    engine_pixels: &[u8],
) -> Result<Outlined, String> {
    use pm_protocol::{color::Precision, common::Size};
    use pm_reference::compare::{Class, dilate};
    use pm_reference::composite::{Blend, Operator, blend, porter_duff};

    let doc = &project.document;
    let (comp_id, resolver) = (project.composition.as_str(), &project.resolver);
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
    let facts = support::text_group_reference::LayerFacts {
        document: doc,
        composition: comp_id,
        time: support::ticks(time),
        effects: renderer.effects(),
    };
    for draw in &plan.draws {
        let ideal = support::text_group_reference::render_draw(draw, size.0, size.1, &facts)?;
        if ideal.is_some() {
            seen_text = true;
        } else if seen_text {
            nontext_above_text = true;
        }
        text_draws.push(ideal);
    }
    let reference = |doc: &pm_model::Document| {
        support::reference::render(
            doc,
            comp_id,
            support::ticks(time),
            size.0,
            size.1,
            true,
            16,
            renderer.effects(),
            resolver,
        )
    };
    let full_hidden = (seen_text && !nontext_above_text).then(|| reference(hidden));
    let background = support::text::background_only(hidden);
    // Why the text is placed over the background alone, for the gap below.
    let mut underlay = String::new();
    let mut base = match full_hidden {
        Some(full) if full.unmodelled.is_empty() && full.modelled() == 1.0 => full,
        full => {
            underlay = match full {
                None => "; nontext drawn above text".into(),
                Some(full) if full.unmodelled.is_empty() => "; nontext partly unmodelled".into(),
                Some(full) => format!("; nontext reference: {}", full.unmodelled.join(", ")),
            };
            reference(&background)
        }
    };
    if !base.unmodelled.is_empty() {
        return Err(base.unmodelled.join("; "));
    }
    // Values an effect's formula leaves open below the text take the value
    // nearest the engine's own frame without text.
    let hidden_unit: Vec<f64> = hidden_pixels.iter().map(|v| f64::from(*v) / 255.0).collect();
    let background_pixels = base.opaque_resolved(&hidden_unit).rgba;
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
        // A top-level text layer's transition runs on the stack below it
        // and the stack with it (`runTransition`).
        let transition = match &draw.content {
            pm_render::plan::Content::Composite(node) => match node.as_ref() {
                pm_render::composite::Node::Layer(layer) if layer.transition.is_some() => {
                    Some(facts.transition(&layer.layer)?.ok_or("inactive text layer transition")?)
                }
                _ => None,
            },
            _ => None,
        };
        let before = transition.as_ref().map(|_| base.rgba.clone());
        for i in 0..w * h {
            let s = &ideal.rgba[i * 4..][..4];
            let d = &mut base.rgba[i * 4..][..4];
            // Effects write light at zero alpha too (Chroma's fringes).
            let lit = s.iter().any(|v| *v != 0.0) || ideal.open[i].is_some();
            if lit && ideal.classes[i] != Class::Unreferenced {
                let background_delta = (0..4)
                    .map(|c| {
                        (f64::from(hidden_pixels[i * 4 + c]) / 255.0 - background_pixels[i * 4 + c])
                            .abs()
                            * 255.0
                    })
                    .fold(0.0f64, f64::max);
                if background_delta > 1.0 + 1e-9 {
                    return Err(format!(
                        "nontext content intersects text at pixel {i}: background delta {background_delta:.2} CV{underlay}"
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
            if !lit {
                continue;
            }
            // Normal is premultiplied source-over, which keeps light at zero
            // alpha; the W3C form drops it there and equals it elsewhere.
            let over = |src: [f64; 4], dst: [f64; 4]| {
                if mode == Blend::Normal {
                    porter_duff(Operator::SourceOver, src, dst)
                } else {
                    blend(mode, src, dst)
                }
            };
            let dst = [d[0], d[1], d[2], d[3]];
            let src = [s[0], s[1], s[2], s[3]];
            base.open[i] = support::reference::composite_open(
                src,
                ideal.open[i].as_ref(),
                dst,
                base.open[i].as_ref(),
                over,
            );
            let result = over(src, dst);
            d.copy_from_slice(&result);
            text_reached[i] = true;
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
        if let (Some((id, params, edge_in, progress)), Some(before)) = (transition, before) {
            let moved = support::reference::apply_transition(
                &mut base, before, &id, &params, edge_in, progress,
            );
            for i in (0..w * h).filter(|&i| moved[i]) {
                text_reached[i] = true;
                if unknown_text[i] {
                    continue;
                }
                if base.rgba[i * 4 + 3].is_nan() {
                    unknown_text[i] = true;
                    classes[i] = Class::Unreferenced;
                } else if base.resampled[i] || base.open[i].is_some() {
                    classes[i] = Class::Resampled;
                } else if classes[i] == Class::Unreferenced {
                    classes[i] = Class::Interior;
                }
            }
        }
    }
    for (i, open) in base.open.iter().enumerate() {
        if open.is_some() && classes[i] != Class::Unreferenced {
            classes[i] = Class::Resampled;
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
                "nontext content intersects text halo at pixel {i}: background delta {background_delta:.2} CV{underlay}"
            ));
        }
        // The reference has only independently modeled background here;
        // any engine ink in this near-outline halo is a real edge error.
        classes[i] = Class::Edge;
    }
    let engine: Vec<f64> = engine_pixels.iter().map(|v| f64::from(*v) / 255.0).collect();
    Ok(Outlined {
        rgba: base.opaque_resolved(&engine).rgba,
        classes,
        reach,
    })
}

/// A text reference gap, judged after the isolation captures are read.
#[cfg(target_os = "macos")]
struct GapObservation {
    pair: String,
    in_isolation: bool,
    over2: usize,
    outside_mask_band: usize,
    /// Its row in the `PM_TEXT_REPORT` verdicts.
    row: usize,
    entry: pending::Entry,
}

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_protocol::common::Size;
    use pm_render::parity::dilate;
    use pm_render::{RenderRequest, Renderer};
    use sha2::{Digest, Sha256};
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
    support::text::configure_coverage(&mut renderer);
    // The built-in effects, transitions, and extension layers as the kernel
    // snapshots them (as tests/corpus.rs sets them).
    renderer
        .set_definitions(&pm_render::all_builtin_definitions())
        .expect("built-in definitions");
    let mut compared = 0;
    let mut failed = Vec::new();
    let mut gate_rows = Vec::new();
    let mut isolated_proofs = std::collections::BTreeMap::new();
    let mut gap_observations = Vec::new();
    let mut pending = pending::Audit::read(include_str!("fixtures/text-corpus-pending.json"));
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
            let authored_text =
                text::authored_isolation_fingerprint(&p.document).expect("authored text");
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
                let ideal = outlined_frame(&mut renderer, &p, &hidden, s.time, (w, h), &base, &ours);
                let mut why = Vec::new();
                let mut gap = None;
                let mut modeled_reach = None;
                let mut unreferenced_text = 0usize;
                let mut reference_metrics = serde_json::Value::Null;
                match ideal {
                    Ok(Outlined {
                        rgba,
                        classes,
                        reach: reached,
                    }) => {
                        use pm_reference::compare::{Bounds, Class, compare};
                        let e: Vec<f64> = ours.iter().map(|x| f64::from(*x) / 255.0).collect();
                        let t: Vec<f64> = reference.iter().map(|x| f64::from(*x) / 255.0).collect();
                        let report = compare(&e, &t, &rgba, &classes);
                        // Modeled text is held to one CV at each pixel. An
                        // interior pixel keeps today's 1 CV bar or proves its
                        // corrected paint against the reference directly
                        // (`quality::reference_problems`, as G-C2). Nontext
                        // is judged through `rest_ok` below.
                        why.extend(
                            quality::reference_problems(
                                &report,
                                &Bounds {
                                    per_pixel_cv: (1.0, 1.0),
                                    ..Bounds::EIGHT_BIT
                                },
                                &e,
                                &t,
                                &rgba,
                                &classes,
                            )
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
                // An inactive text layer has a known transparent source.
                // Only accept that interpretation when neither witness nor
                // engine contributes any text pixel; the complete frame
                // remains under the original nontext comparison below.
                let inactive_text = gap.as_deref() == Some("no active text") && n == 0;
                if inactive_text {
                    gap = None;
                }
                // With a modeled scene, the independent ink footprint—not
                // differences from the hidden engine frame—defines text.
                // Thus unrelated nontext differences cannot disappear into
                // the text region of the old comparison.
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
                let rest: Vec<bool> = modeled_reach
                    .as_ref()
                    .unwrap_or(&mask)
                    .iter()
                    .zip(&bands)
                    .map(|(m, b)| !m && !b)
                    .collect();
                let d = Diff::of_masked(&ours, &reference, &rest);
                // Non-text pixels use the same authored f64 proof as G-C2:
                // corrected vector coverage is judged against its outline,
                // not against the old density-bucket raster in the capture.
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
                let opaque = r.opaque_resolved(
                    &ours
                        .iter()
                        .map(|v| f64::from(*v) / 255.0)
                        .collect::<Vec<_>>(),
                );
                let selected = |values: &[f64]| {
                    values
                        .chunks_exact(4)
                        .zip(&rest)
                        .filter(|(_, keep)| **keep)
                        .flat_map(|(px, _)| px.iter().copied())
                        .collect::<Vec<_>>()
                };
                let unit = |px: &[u8]| px.iter().map(|v| f64::from(*v) / 255.0).collect::<Vec<_>>();
                let (engine_full, today_full) = (unit(&ours), unit(&reference));
                let (engine, today) = (selected(&engine_full), selected(&today_full));
                let wanted = selected(&opaque.rgba);
                let classes: Vec<_> = opaque
                    .classes()
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
                let owner = |reasons: &[String]| {
                    if reasons.iter().any(|r| r.contains("effect") || r.contains("transition")
                        || r.contains("motion-blurred") || r.contains("matte"))
                    {
                        "L6"
                    } else {
                        "L4"
                    }
                };
                let label = format!("{}{tag}", p.name);
                let rest_owned = pending.check(
                    pending::Entry {
                        frame: pending::frame_id(&label, s.time, w, h, "non-text"),
                        owner: owner(&r.unmodelled).into(),
                        reason: r.unmodelled.join(", "),
                        checks: problems.clone(),
                        measurement_sha256: pending::measurement(&engine, &today, &wanted, &classes),
                    },
                    !r.unmodelled.is_empty() && problems.iter().all(|p| p.starts_with("unreferenced:")),
                );
                let rest_ok = problems.is_empty() || rest_owned;
                // Text the reference cannot model yet (effects, projection,
                // shared groups) may become an owned gap only with
                // independent evidence that its glyph raster is right: the
                // same authored text, time and size passes the outline
                // reference in isolation, or every pixel over the original
                // 2 CV bar lies in an independently judged mask band.
                // Isolation is measured last, so admission is deferred.
                let pair = pending::frame_id(
                    &format!("{}:{authored_text}", p.name),
                    s.time,
                    w,
                    h,
                    "text-pair",
                );
                let outside_mask_band = (0..wu * hu)
                    .filter(|&i| mask[i] && !bands[i])
                    .filter(|&i| (0..4).any(|c| ours[i * 4 + c].abs_diff(reference[i * 4 + c]) > 2))
                    .count();
                if in_isolation && gap.is_none() && why.is_empty() {
                    isolated_proofs.insert(pair.clone(), format!("{:x}", Sha256::digest(&ours)));
                }
                compared += 1;
                if let Some(reason) = gap.as_ref().filter(|_| why.is_empty() && rest_ok) {
                    gap_observations.push(GapObservation {
                        pair,
                        in_isolation,
                        over2,
                        outside_mask_band,
                        row: gate_rows.len(),
                        entry: pending::Entry {
                            frame: pending::frame_id(&label, s.time, w, h, "text"),
                            owner: owner(std::slice::from_ref(reason)).into(),
                            reason: reason.clone(),
                            checks: vec![format!(
                                "text reference gap; original bar: {over2} pixels over 2 CV, max {tmax}, outside independent mask band {outside_mask_band}"
                            )],
                            measurement_sha256: pending::measurement(
                                &engine_full,
                                &today_full,
                                &opaque.rgba,
                                &opaque.classes(),
                            ),
                        },
                    });
                }
                let deferred = gap.is_some() && why.is_empty() && rest_ok;
                let fails = gap.is_some() || !why.is_empty() || !rest_ok;
                exact += usize::from(!fails && gap.is_none());
                let frame_index = compared - 1;
                let engine_sha256 = format!("{:x}", Sha256::digest(&ours));
                gate_rows.push(serde_json::json!({
                    "case": frame_index, "pass": !fails && gap.is_none(),
                    "ownedPending": rest_ok && !problems.is_empty(),
                    "width": w, "height": h,
                    "captureDir": capture.display().to_string(),
                    "projectDir": dir.strip_prefix(capture).unwrap().display().to_string(),
                    "sampleIndex": sample_index, "time": s.time,
                    "engineSha256": engine_sha256,
                    "referenceModel": if inactive_text { "inactive-text-original-bound" }
                        else if gap.is_some() { "original-bound-gap" }
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
                        .map_or("exact outline reference", |_| "reference gap")
                );
                println!("{line}\n    non-text f64 reference{proof}");
                for problem in &problems {
                    println!("    {problem}");
                }
                if fails && !deferred {
                    if !why.is_empty() {
                        println!("  frame {} why: {}", compared - 1, why.join("; "));
                    }
                    failed.push(format!("{line}: {}", why.iter().chain(&problems).cloned().collect::<Vec<_>>().join("; ")));
                }
                if !fails && gap.is_none() && !inactive_text {
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
                    // The non-text reference; unreferenced pixels in magenta.
                    let wanted: Vec<u8> = opaque
                        .rgba
                        .chunks_exact(4)
                        .flat_map(|c| {
                            if c[3].is_nan() {
                                [255, 0, 255, 255]
                            } else {
                                let b = |v: f64| (v * 255.0).round() as u8;
                                [b(c[0]), b(c[1]), b(c[2]), 255]
                            }
                        })
                        .collect();
                    write_png(&dir.join(format!("{stem}-reference.png")), w, h, &wanted);
                }
            }
        }
    }
    let mut text_pending = 0;
    for gap in gap_observations {
        let paired = isolated_proofs.get(&gap.pair);
        let mut entry = gap.entry;
        if let Some(proof) = paired {
            entry.reason = format!("{} with passing authored-isolation outline reference", entry.reason);
            entry.checks.push(format!("paired isolated text passes; engine {proof}"));
        } else if gap.over2 > 0 && gap.outside_mask_band == 0 {
            entry.owner = "L4".into();
            entry.reason = format!("{}; every text pixel over 2 CV is in the independent mask band", entry.reason);
        }
        let eligible = quality::text_context_is_owned_gap(
            gap.in_isolation,
            paired.is_some(),
            gap.over2,
            gap.outside_mask_band,
        );
        let owned = pending.check(entry, eligible);
        gate_rows[gap.row]["ownedPending"] = owned.into();
        if owned {
            text_pending += 1;
        } else {
            failed.push(format!("frame {}: unowned text reference gap", gate_rows[gap.row]["case"]));
        }
    }
    println!("owned text reference gaps: {text_pending}");
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
