//! G-T1: captured text rasters against independent f64 outline coverage.
//! The captures witness today's output; exact layout is guarded separately.

mod support;

/// Reference for the SDR sRGB8 presentation of an extended premultiplied
/// Standard-domain float pixel. CSS Color 4 maps the straight color before
/// alpha is applied; RGBA8 quantization then occurs on the premultiplied
/// result. The unclipped float raster is checked separately below.
#[cfg(target_os = "macos")]
fn display_srgb8(rgba: &[f64], gamut_map: impl Fn([f64; 3]) -> [f64; 3]) -> Vec<f64> {
    rgba.chunks_exact(4)
        .flat_map(|p| {
            if p.iter().any(|v| v.is_nan()) {
                return [f64::NAN; 4];
            }
            let a = p[3].clamp(0.0, 1.0);
            let mapped = if a > 0.0 {
                gamut_map([p[0] / a, p[1] / a, p[2] / a])
            } else {
                [0.0; 3]
            };
            let premul = [mapped[0] * a, mapped[1] * a, mapped[2] * a, a];
            premul.map(|v| (v.clamp(0.0, 1.0) * 255.0).round_ties_even() / 255.0)
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn needs_srgb_gamut_mapping(p: &[f64]) -> bool {
    let a = p[3];
    a > 0.0
        && p[..3]
            .iter()
            .any(|channel| !((0.0..=1.0).contains(&(*channel / a))))
}

#[cfg(target_os = "macos")]
struct SelectedReference {
    image: support::text_reference::TextReference,
    samples: usize,
    geometry: serde_json::Value,
    failures: Vec<String>,
}

#[cfg(target_os = "macos")]
fn max_modeled_raw_cv(actual: &[f64], reference: &[f64]) -> f64 {
    assert_eq!(actual.len(), reference.len());
    actual
        .chunks_exact(4)
        .zip(reference.chunks_exact(4))
        .filter(|(_, reference)| reference.iter().all(|v| v.is_finite()))
        .flat_map(|(actual, reference)| {
            (0..4).map(move |c| {
                if actual[c].is_finite() {
                    (actual[c] - reference[c]).abs() * 255.0
                } else {
                    f64::INFINITY
                }
            })
        })
        .fold(0.0, f64::max)
}

/// Independent convergence between two complete f64 reference images. No
/// engine pixel or pixel class participates in the selection decision.
#[cfg(target_os = "macos")]
fn whole_reference_delta_cv(previous: &[f64], next: &[f64]) -> Result<f64, String> {
    assert_eq!(previous.len(), next.len());
    let mut max_cv = 0.0f64;
    for (i, (&a, &b)) in previous.iter().zip(next).enumerate() {
        if a.is_nan() && b.is_nan() {
            continue;
        }
        if !a.is_finite() || !b.is_finite() {
            return Err(format!("reference modeled channel status changed at {i}"));
        }
        max_cv = max_cv.max((a - b).abs() * 255.0);
    }
    Ok(max_cv)
}

#[cfg(target_os = "macos")]
fn select_reference(
    scene: &pm_text::render::scene::TextScene,
    engine: &[f64],
    width: usize,
) -> SelectedReference {
    use pm_protocol::color::WorkingEncoding;
    use pm_reference::compare::{Geometry, geometry, same_geometry};

    let report_geometry =
        |g: Geometry| serde_json::json!({ "area": g.area, "centroid": g.centroid });
    let reference32 = support::text_reference::render(scene);
    if reference32.unreferenced != 0 {
        return SelectedReference {
            image: reference32,
            samples: 32,
            geometry: serde_json::json!({
                "status": "unreferenced image pixels",
                "convergenceStatus": "not_applicable",
            }),
            failures: Vec::new(),
        };
    }
    let engine_alpha: Vec<f64> = engine.chunks_exact(4).map(|p| p[3]).collect();
    let actual = geometry(&engine_alpha, width);
    let alpha32: Vec<f64> = reference32.rgba.chunks_exact(4).map(|p| p[3]).collect();
    let g32 = geometry(&alpha32, width);
    let first_geometry = same_geometry(&actual, &g32, 1e-3);
    let raw32_max_cv = max_modeled_raw_cv(engine, &reference32.rgba);
    if first_geometry.is_ok() && raw32_max_cv <= 1.0 + 1e-9 {
        return SelectedReference {
            image: reference32,
            samples: 32,
            geometry: serde_json::json!({
                "engine": report_geometry(actual),
                "reference32": report_geometry(g32),
                "raw32MaxCv": raw32_max_cv,
                "selectedPass": true,
                "selectedTolerance": 1e-3,
                "convergenceStatus": "not_required",
            }),
            failures: Vec::new(),
        };
    }
    // A 32-cell quadrature miss can be in geometry or an individual pixel.
    // Integrate complete images until their geometry and all modeled RGBA
    // channels converge independently of the engine; the final whole image
    // then receives every unchanged gate rule.
    let first_error = first_geometry.err();
    drop(engine_alpha);
    drop(alpha32);
    drop(reference32);
    let mut previous =
        support::text_reference::render_with_samples(scene, WorkingEncoding::Standard, 256);
    let alpha256: Vec<f64> = previous.rgba.chunks_exact(4).map(|p| p[3]).collect();
    let g256 = geometry(&alpha256, width);
    drop(alpha256);
    let mut previous_samples = 256;
    let mut previous_geometry = g256;
    let mut steps = Vec::new();
    let (image, samples, selected_geometry, geometry_convergence, pixel_convergence) = loop {
        let samples = previous_samples * 2;
        let candidate =
            support::text_reference::render_with_samples(scene, WorkingEncoding::Standard, samples);
        let candidate_alpha: Vec<f64> = candidate.rgba.chunks_exact(4).map(|p| p[3]).collect();
        let candidate_geometry = geometry(&candidate_alpha, width);
        let geometry_verdict = same_geometry(&previous_geometry, &candidate_geometry, 1e-4);
        let pixel_verdict = whole_reference_delta_cv(&previous.rgba, &candidate.rgba);
        let pixel_pass = pixel_verdict
            .as_ref()
            .is_ok_and(|delta| *delta <= 0.01 + 1e-9);
        let converged = geometry_verdict.is_ok() && pixel_pass;
        steps.push(serde_json::json!({
            "fromSamples": previous_samples,
            "toSamples": samples,
            "from": report_geometry(previous_geometry),
            "to": report_geometry(candidate_geometry),
            "geometryPass": geometry_verdict.is_ok(),
            "geometryFailure": geometry_verdict.as_ref().err(),
            "pixelMaxCv": pixel_verdict.as_ref().ok(),
            "pixelPass": pixel_pass,
            "pixelFailure": pixel_verdict.as_ref().err(),
            "pass": converged,
        }));
        if converged || samples == 2048 {
            break (
                candidate,
                samples,
                candidate_geometry,
                geometry_verdict,
                pixel_verdict,
            );
        }
        previous = candidate;
        previous_samples = samples;
        previous_geometry = candidate_geometry;
    };
    let selected = same_geometry(&actual, &selected_geometry, 1e-3);
    let mut failures = Vec::new();
    if let Err(reason) = &geometry_convergence {
        failures.push(format!(
            "independent reference geometry did not converge by {samples} cells: {reason}"
        ));
    }
    match &pixel_convergence {
        Ok(delta) if *delta > 0.01 + 1e-9 => failures.push(format!(
            "independent reference pixels did not converge by {samples} cells: {delta:.6} CV (bound 0.01)"
        )),
        Err(reason) => failures.push(format!(
            "independent reference pixels did not converge by {samples} cells: {reason}"
        )),
        _ => {}
    }
    if let Err(reason) = &selected {
        failures.push(reason.clone());
    }
    SelectedReference {
        image,
        samples,
        geometry: serde_json::json!({
            "engine": report_geometry(actual),
            "reference32": report_geometry(g32),
            "reference256": report_geometry(g256),
            "raw32MaxCv": raw32_max_cv,
            "first32Failure": first_error,
            "convergenceSteps": steps,
            "convergencePass": geometry_convergence.is_ok()
                && pixel_convergence.as_ref().is_ok_and(|delta| *delta <= 0.01 + 1e-9),
            "geometryConvergenceTolerance": 1e-4,
            "pixelConvergenceCv": 0.01,
            "geometryConvergenceFailure": geometry_convergence.err(),
            "pixelConvergenceFailure": pixel_convergence.err(),
            "selectedSamples": samples,
            "selectedReference": report_geometry(selected_geometry),
            "selectedPass": selected.is_ok(),
            "selectedTolerance": 1e-3,
            "selectedFailure": selected.err(),
        }),
        failures,
    }
}

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_reference::compare::{Bounds, Class, compare};
    use pm_render::Renderer;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use support::doc::{layer, project};

    verify_display_boundary();

    let sets: Vec<std::path::PathBuf> = match std::env::var("PM_TEXT_RASTER") {
        Ok(p) => vec![p.into()],
        Err(_) => [
            "text-raster-",
            "text-closure-",
            "text-paths-",
            "text-source-",
            "text-transformed-",
        ]
        .iter()
        .filter_map(|p| support::newest_capture(p))
        .collect(),
    };
    if sets.is_empty() && support::corpus_root().is_none() {
        eprintln!("no private corpus directory: G-T1 has no captured witnesses");
        return;
    }
    let mut failures = Vec::new();
    let (mut seen, mut image_pixels) = (0, 0);
    let mut rows = Vec::new();
    for (set_index, dir) in sets.into_iter().enumerate() {
        let cases: Value =
            serde_json::from_slice(&std::fs::read(dir.join("cases.json")).unwrap()).unwrap();
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        let corpus = pm_text::corpus::CorpusFile::from_value(&json!({
            "variableSources": cases["variableSources"], "projectFonts": cases["projectFonts"], "cases": [],
        })).unwrap();
        let registry = pm_text::corpus::native_registry(&corpus).expect("registry");
        let mut measure = pm_eval::text_measure::CompatMeasure::new(registry);
        if let Some(seeds) = cases.get("seeds") {
            measure
                .compat()
                .set_font_cache_seeds(serde_json::from_value(seeds.clone()).unwrap());
        }
        let mut renderer = Renderer::headless().expect("Metal device");
        support::text::configure_coverage(&mut renderer);
        renderer.set_text_fonts(Some(Box::new(support::text::MainThreadFonts(
            measure, None,
        ))));
        let only = std::env::var("PM_TEXT_ONLY").ok();
        let capture_case_only = std::env::var("PM_TEXT_CAPTURE_CASE_INDEX")
            .ok()
            .and_then(|value| value.parse::<usize>().ok());
        for (capture_case_index, (case, shot)) in cases["cases"]
            .as_array()
            .unwrap()
            .iter()
            .zip(manifest["cases"].as_array().unwrap())
            .enumerate()
        {
            let id = case["id"].as_str().unwrap();
            assert_eq!(shot["id"].as_str(), Some(id));
            if only
                .as_deref()
                .is_some_and(|prefix| !id.starts_with(prefix))
                || capture_case_only.is_some_and(|selected| capture_case_index != selected)
                || shot.get("empty").is_some()
            {
                continue;
            }
            let index = seen;
            seen += 1;
            let (doc, layer_id, time) = if case["kind"] == "text" {
                (
                    project(
                        1920,
                        1080,
                        10.0,
                        support::doc::no_fill(),
                        vec![layer("t", "text", case["d"].clone(), 0.0, 0.0, 10.0)],
                    ),
                    "t",
                    0.0,
                )
            } else {
                let w = case["compositionWidth"].as_u64().unwrap() as u32;
                let h = case["compositionHeight"].as_u64().unwrap() as u32;
                (
                    project(
                        w,
                        h,
                        10.0,
                        support::doc::no_fill(),
                        vec![layer(
                            "c",
                            "captions",
                            case["content"].clone(),
                            960.0,
                            540.0,
                            10.0,
                        )],
                    ),
                    "c",
                    case["local"].as_f64().unwrap(),
                )
            };
            let doc =
                pm_model::Document::new(Some(pm_model::json::parse(&doc.to_string()).unwrap()), 1);
            let (w, h) = (
                shot["width"].as_u64().unwrap() as usize,
                shot["height"].as_u64().unwrap() as usize,
            );
            let today = std::fs::read(dir.join(format!("{id}.rgba"))).unwrap();
            assert_eq!(today.len(), w * h * 4, "case {index}: witness size");
            let raster = match renderer.text_raster(
                &doc,
                "Ptest-root",
                layer_id,
                time,
                case["scale"].as_f64().unwrap(),
                Precision::Float32,
            ) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    failures.push(format!("case {index}: no ink"));
                    continue;
                }
                Err(e) => {
                    failures.push(format!("case {index}: {e}"));
                    continue;
                }
            };
            if raster.canvas != (w as u32, h as u32) {
                failures.push(format!(
                    "case {index}: canvas {:?} vs {w}x{h}",
                    raster.canvas
                ));
                continue;
            }
            let mut engine = vec![0.0; w * h * 4];
            let [x0, y0, x1, y1] = raster.plan.crop.map(|v| v as usize);
            let crop_w = x1 - x0;
            for y in y0..y1 {
                for x in x0..x1 {
                    for c in 0..4 {
                        engine[(y * w + x) * 4 + c] =
                            f64::from(raster.pixels[((y - y0) * crop_w + x - x0) * 4 + c]);
                    }
                }
            }
            let today: Vec<f64> = today.iter().map(|v| f64::from(*v) / 255.0).collect();
            let SelectedReference {
                image: mut ideal,
                samples: reference_samples,
                geometry: geometry_report,
                failures: geometry_failures,
            } = select_reference(&raster.plan.scene, &engine, w);
            if reference_samples > 32 {
                println!(
                    "case {index}: 32-cell geometry or raw-pixel miss; selected converged {reference_samples}-cell whole-image verdict: {geometry_report}"
                );
            }
            // The legacy capture can put partially covered ink outside the
            // exact outline's two-pixel edge band (CoreText's old padding
            // and preblend). A pixel-aligned exact edge has no fractional
            // coverage pixel, so include both images' support boundaries.
            // An opaque caption box can hide a colored glyph edge in final
            // alpha; a witness RGBA change between neighbors still marks
            // that local color boundary. Uniform interiors remain interior.
            let support: Vec<bool> = today.chunks_exact(4).map(|p| p[3] > 0.5 / 255.0).collect();
            let exact_support: Vec<bool> = ideal.rgba.chunks_exact(4).map(|p| p[3] > 0.0).collect();
            let mut support_boundary = vec![false; w * h];
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    support_boundary[i] =
                        [(&support, i), (&exact_support, i)]
                            .into_iter()
                            .any(|(pixels, i)| {
                                (x > 0 && pixels[i] != pixels[i - 1])
                                    || (x + 1 < w && pixels[i] != pixels[i + 1])
                                    || (y > 0 && pixels[i] != pixels[i - w])
                                    || (y + 1 < h && pixels[i] != pixels[i + w])
                            })
                            || [
                                (x > 0).then_some(i.wrapping_sub(1)),
                                (x + 1 < w).then_some(i + 1),
                                (y > 0).then_some(i.wrapping_sub(w)),
                                (y + 1 < h).then_some(i + w),
                            ]
                            .into_iter()
                            .flatten()
                            .any(|neighbor| {
                                today[i * 4..i * 4 + 4] != today[neighbor * 4..neighbor * 4 + 4]
                            });
                }
            }
            for (class, edge) in ideal.classes.iter_mut().zip(pm_reference::compare::dilate(
                &support_boundary,
                w,
                h,
                2,
            )) {
                if edge && *class == Class::Interior {
                    *class = Class::Edge;
                }
            }
            image_pixels += ideal.image_pixels;
            // Keep the native float result as the transport attestation and
            // require independent raw f64 closeness before any SDR gamut map.
            let raw_nonfinite = engine
                .chunks_exact(4)
                .zip(ideal.rgba.chunks_exact(4))
                .filter(|(_, reference)| reference.iter().all(|v| v.is_finite()))
                .filter(|(actual, _)| actual.iter().any(|v| !v.is_finite()))
                .count();
            let raw_errors: Vec<f64> = engine
                .chunks_exact(4)
                .zip(ideal.rgba.chunks_exact(4))
                .filter(|(actual, reference)| {
                    actual.iter().all(|v| v.is_finite()) && reference.iter().all(|v| v.is_finite())
                })
                .map(|(actual, reference)| {
                    (0..4)
                        .map(|c| (actual[c] - reference[c]).abs() * 255.0)
                        .fold(0.0f64, f64::max)
                })
                .collect();
            let raw_stats = pm_reference::compare::Stats::of(raw_errors);
            let display_engine = display_srgb8(&engine, pm_color::css_srgb);
            let display_reference =
                display_srgb8(&ideal.rgba, pm_reference::color::css_gamut_map_srgb);
            let report = compare(&display_engine, &today, &display_reference, &ideal.classes);
            let display_bytes: Vec<u8> = display_engine
                .iter()
                .map(|v| (v * 255.0).round() as u8)
                .collect();
            let engine_bytes: Vec<u8> = engine
                .iter()
                .map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect();
            let engine_sha256 = format!("{:x}", Sha256::digest(&engine_bytes));
            let mut why = report.failures(&Bounds {
                per_pixel_cv: (1.0, 1.0),
                ..Bounds::EIGHT_BIT
            });
            // CSS Color 4 requires a destination gamut map for these
            // authored extended colors. Today's pre-mapping capture is a
            // witness, not the color target. Keep the same 1-CV interior
            // limit against the independent mapped reference, pixel by
            // pixel; ordinary interiors retain their 1-CV witness rule.
            why.retain(|issue| !issue.starts_with("interior:"));
            let mut ordinary_interior = Vec::new();
            let mut gamut_interior = Vec::new();
            for (i, class) in ideal.classes.iter().enumerate() {
                if *class != Class::Interior {
                    continue;
                }
                let source = &ideal.rgba[i * 4..i * 4 + 4];
                if source.iter().any(|v| v.is_nan()) {
                    continue;
                }
                let actual = &display_engine[i * 4..i * 4 + 4];
                let gamut_mapped = needs_srgb_gamut_mapping(source);
                let target = if gamut_mapped {
                    &display_reference[i * 4..i * 4 + 4]
                } else {
                    &today[i * 4..i * 4 + 4]
                };
                let cv = (0..4)
                    .map(|c| (actual[c] - target[c]).abs() * 255.0)
                    .fold(0.0f64, f64::max);
                if gamut_mapped {
                    gamut_interior.push(cv);
                } else {
                    ordinary_interior.push(cv);
                }
            }
            let ordinary_interior = pm_reference::compare::Stats::of(ordinary_interior);
            let gamut_interior = pm_reference::compare::Stats::of(gamut_interior);
            if ordinary_interior.max > 1.0 + 1e-9 {
                why.push(format!(
                    "interior: {:.3} CV from today (bound 1)",
                    ordinary_interior.max
                ));
            }
            if gamut_interior.max > 1.0 + 1e-9 {
                why.push(format!(
                    "gamut-mapped interior: {:.3} CV from independent CSS reference (bound 1)",
                    gamut_interior.max
                ));
            }
            if raw_stats.max > 1.0 + 1e-9 {
                why.push(format!(
                    "raw float raster exceeds independent f64 outline by {:.3} CV (bound 1)",
                    raw_stats.max
                ));
            }
            if raw_nonfinite > 0 {
                why.push(format!(
                    "raw float raster has {raw_nonfinite} nonfinite modeled pixels"
                ));
            }
            why.extend(geometry_failures);
            if report.class(Class::Unreferenced).engine_vs_today.max > 1.0 {
                why.push("unmodeled float image pixels exceed original 1-CV bound".into());
            }
            let edge = report.class(Class::Edge);
            let image = report.class(Class::Resampled);
            let stat = |s: &pm_reference::compare::Stats| json!({ "max": s.max, "p999": s.p999, "mean": s.mean, "pixels": s.pixels });
            println!(
                "case {index}: edge engine {} / today {}; image engine {} / today {}; {}",
                edge.engine_vs_reference,
                edge.today_vs_reference,
                image.engine_vs_reference,
                image.today_vs_reference,
                if why.is_empty() { "PASS" } else { "FAIL" }
            );
            for issue in &why {
                println!("case {index} why: {issue}");
            }
            if why.iter().any(|issue| issue.starts_with("interior:")) {
                for (i, _) in ideal
                    .classes
                    .iter()
                    .enumerate()
                    .filter(|(_, class)| **class == Class::Interior)
                {
                    let actual = &display_engine[i * 4..][..4];
                    let witness = &today[i * 4..][..4];
                    let reference = &display_reference[i * 4..][..4];
                    let original_cv = (0..4)
                        .map(|c| (actual[c] - witness[c]).abs() * 255.0)
                        .fold(0.0f64, f64::max);
                    if original_cv <= 1.0 {
                        continue;
                    }
                    let modeled_cv = (0..4)
                        .map(|c| (actual[c] - reference[c]).abs() * 255.0)
                        .fold(0.0f64, f64::max);
                    println!(
                        "case {index} display interior residual pixel ({},{}): original {original_cv:.3} CV; mapped outline {modeled_cv:.3} CV; alpha engine {:.6} witness {:.6} reference {:.6}",
                        i % w,
                        i / w,
                        actual[3],
                        witness[3],
                        reference[3]
                    );
                    break;
                }
            }
            rows.push(json!({
                "case": index, "pass": why.is_empty(), "width": w, "height": h,
                "setIndex": set_index, "captureCaseIndex": capture_case_index,
                "captureDir": dir.display().to_string(),
                "engineSha256": engine_sha256,
                "displaySha256": format!("{:x}", Sha256::digest(&display_bytes)),
                "referenceModel": if ideal.unreferenced == 0 { "outline" } else { "outline-with-original-bound-gap" },
                "referenceSamples": reference_samples,
                "geometry": geometry_report,
                "displayBoundary": "css-color-4-oklch-local-minde-srgb8-premultiplied",
                "rawFloat": stat(&raw_stats),
                "rawNonfinitePixels": raw_nonfinite,
                "ordinaryInterior": stat(&ordinary_interior),
                "gamutInterior": stat(&gamut_interior),
                "imagePixels": ideal.image_pixels,
                "unreferencedPixels": ideal.unreferenced,
                "edge": { "engine": stat(&edge.engine_vs_reference),
                    "today": stat(&edge.today_vs_reference),
                    "perPixelExcess": stat(&edge.excess) },
                "image": { "engine": stat(&image.engine_vs_reference),
                    "today": stat(&image.today_vs_reference),
                    "perPixelExcess": stat(&image.excess) },
                "interiorToday": stat(&report.class(Class::Interior).engine_vs_today),
                "unreferencedToday": stat(&report.class(Class::Unreferenced).engine_vs_today),
                "failures": &why,
            }));
            for (variable, extension) in [("PM_TEXT_DIFFS", "png"), ("PM_TEXT_DUMP", "rgba")] {
                if let Ok(dir) = std::env::var(variable) {
                    let dir = std::path::Path::new(&dir);
                    assert!(
                        !dir.starts_with(support::repo_root()),
                        "text evidence must stay outside Git"
                    );
                    if !why.is_empty() || variable == "PM_TEXT_DUMP" {
                        std::fs::create_dir_all(dir).unwrap();
                        let to8 = |v: &[f64]| -> Vec<u8> {
                            v.iter()
                                .map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)
                                .collect()
                        };
                        let actual = engine_bytes.clone();
                        let witness = to8(&today);
                        let reference = to8(&ideal.rgba);
                        if extension == "rgba" {
                            std::fs::write(dir.join(format!("case-{index}-engine.rgba")), actual)
                                .unwrap();
                            std::fs::write(
                                dir.join(format!("case-{index}-display.rgba")),
                                &display_bytes,
                            )
                            .unwrap();
                            std::fs::write(dir.join(format!("case-{index}-today.rgba")), witness)
                                .unwrap();
                            std::fs::write(
                                dir.join(format!("case-{index}-outline.rgba")),
                                reference,
                            )
                            .unwrap();
                        } else {
                            let diff: Vec<u8> = actual
                                .chunks_exact(4)
                                .zip(witness.chunks_exact(4))
                                .flat_map(|(a, b)| {
                                    let d = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap();
                                    [d.saturating_mul(4), 0, 0, 255]
                                })
                                .collect();
                            support::write_png(
                                &dir.join(format!("case-{index}-engine.png")),
                                w as u32,
                                h as u32,
                                &support::text::png_rgba(&actual, true),
                            );
                            support::write_png(
                                &dir.join(format!("case-{index}-today.png")),
                                w as u32,
                                h as u32,
                                &support::text::png_rgba(&witness, true),
                            );
                            support::write_png(
                                &dir.join(format!("case-{index}-outline.png")),
                                w as u32,
                                h as u32,
                                &support::text::png_rgba(&reference, true),
                            );
                            support::write_png(
                                &dir.join(format!("case-{index}-diff.png")),
                                w as u32,
                                h as u32,
                                &diff,
                            );
                        }
                    }
                }
            }
            for issue in why {
                failures.push(format!("case {index}: {issue}"));
            }
        }
    }
    if let Ok(path) = std::env::var("PM_TEXT_REPORT") {
        let path = std::path::Path::new(&path);
        assert!(
            !path.starts_with(support::repo_root()),
            "report must stay outside Git"
        );
        std::fs::write(path, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
    }
    println!(
        "G-T1: {seen} cases; {image_pixels} image pixels; {} failures",
        failures.len()
    );
    for f in &failures {
        println!("  {f}");
    }
    assert!(failures.is_empty(), "G-T1 reference-closeness failures");
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn verify_display_boundary() {
    let raw = [0.0, 0.408, 0.0, 0.4];
    let display = display_srgb8(&raw, pm_reference::color::css_gamut_map_srgb);
    // A per-channel clamp of the premultiplied raw green is 104/255;
    // CSS mapping of the straight 1.02 green before alpha is 102/255.
    assert_eq!((raw[1] * 255.0).round() as u8, 104);
    assert_eq!((display[1] * 255.0).round() as u8, 102);
    assert_eq!((display[3] * 255.0).round() as u8, 102);
    // The gate's target and the production output operation have separate
    // implementations. Exercise their whole out-of-gamut conversion domain
    // before reading private captures, including the translucent sample.
    for r in [-0.5, 0.0, 0.3, 0.7, 1.0, 1.5] {
        for g in [-0.5, 0.0, 0.3, 0.7, 1.0, 1.5] {
            for b in [-0.5, 0.0, 0.3, 0.7, 1.0, 1.5] {
                let color = [r, g, b];
                let actual = pm_color::css_srgb(color);
                let reference = pm_reference::color::css_gamut_map_srgb(color);
                for c in 0..3 {
                    assert!(
                        (actual[c] - reference[c]).abs() * 255.0 <= 1.0,
                        "CSS gamut reference differs at {color:?}: {actual:?} vs {reference:?}"
                    );
                }
            }
        }
    }
}
