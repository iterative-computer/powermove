//! G-T1: captured text rasters against independent f64 outline coverage.
//! The captures witness today's output; exact layout is guarded separately.

mod support;

#[cfg(target_os = "macos")]
struct SelectedReference {
    image: support::text_reference::TextReference,
    samples: usize,
    geometry: serde_json::Value,
    failures: Vec<String>,
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
    if same_geometry(&actual, &g32, 1e-3).is_ok() {
        return SelectedReference {
            image: reference32,
            samples: 32,
            geometry: serde_json::json!({
                "engine": report_geometry(actual),
                "reference32": report_geometry(g32),
                "selectedPass": true,
                "selectedTolerance": 1e-3,
                "convergenceStatus": "not_required",
            }),
            failures: Vec::new(),
        };
    }
    // A 32-cell quadrature miss cannot be accepted on geometry alone.
    // Integrate the *whole image* at two higher resolutions, demand their
    // geometry converge, then use 512-cell pixels for every gate comparison.
    let first_error = same_geometry(&actual, &g32, 1e-3).unwrap_err();
    drop(engine_alpha);
    drop(alpha32);
    drop(reference32);
    let reference256 =
        support::text_reference::render_with_samples(scene, WorkingEncoding::Standard, 256);
    let alpha256: Vec<f64> = reference256.rgba.chunks_exact(4).map(|p| p[3]).collect();
    let g256 = geometry(&alpha256, width);
    drop(alpha256);
    drop(reference256);
    let reference512 =
        support::text_reference::render_with_samples(scene, WorkingEncoding::Standard, 512);
    let alpha512: Vec<f64> = reference512.rgba.chunks_exact(4).map(|p| p[3]).collect();
    let g512 = geometry(&alpha512, width);
    drop(alpha512);
    let convergence = same_geometry(&g256, &g512, 1e-4);
    let selected = same_geometry(&actual, &g512, 1e-3);
    let mut failures = Vec::new();
    if let Err(reason) = &convergence {
        failures.push(format!(
            "independent 256/512 reference did not converge: {reason}"
        ));
    }
    if let Err(reason) = &selected {
        failures.push(reason.clone());
    }
    SelectedReference {
        image: reference512,
        samples: 512,
        geometry: serde_json::json!({
            "engine": report_geometry(actual),
            "reference32": report_geometry(g32),
            "reference256": report_geometry(g256),
            "reference512": report_geometry(g512),
            "first32Failure": first_error,
            "convergencePass": convergence.is_ok(),
            "convergenceTolerance": 1e-4,
            "convergenceFailure": convergence.err(),
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
    use pm_reference::compare::{Bounds, Class, compare, geometry, same_geometry};
    use pm_render::Renderer;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use support::doc::{layer, project};

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
            if reference_samples == 512 {
                println!(
                    "case {index}: 32-cell geometry miss; 256/512 convergence and selected 512-cell verdict: {geometry_report}"
                );
            }
            // The legacy capture can put partially covered ink outside the
            // exact outline's two-pixel edge band (CoreText's old padding
            // and preblend). A pixel-aligned exact edge has no fractional
            // coverage pixel, so include both images' support boundaries.
            // Authored interior colors retain the interior rule.
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
            if index == 0 && std::env::var_os("PM_TEXT_COVERAGE_ORACLE").is_some() {
                for (item, path, area32, winding256) in
                    support::text_reference::point_oracle(&raster.plan.scene, 205, 35)
                {
                    println!(
                        "coverage oracle item {item} path {path} pixel (205,35): 32-cell {area32:.6}, winding-256 {winding256:.6}, delta {:.3} CV",
                        (area32 - winding256).abs() * 255.0
                    );
                }
            }
            image_pixels += ideal.image_pixels;
            let report = compare(&engine, &today, &ideal.rgba, &ideal.classes);
            let engine_bytes: Vec<u8> = engine
                .iter()
                .map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect();
            let engine_sha256 = format!("{:x}", Sha256::digest(&engine_bytes));
            let mut why = report.failures(&Bounds {
                per_pixel_cv: (1.0, 1.0),
                ..Bounds::EIGHT_BIT
            });
            why.extend(geometry_failures);
            if report.class(Class::Unreferenced).engine_vs_today.max > 1.0 {
                why.push("unmodeled float image pixels exceed original 1-CV bound".into());
            }
            if ideal.unreferenced == 0
                && std::env::var("PM_TEXT_REFERENCE_CASE")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    == Some(index)
            {
                let ea: Vec<f64> = engine.chunks_exact(4).map(|p| p[3]).collect();
                let ra: Vec<f64> = ideal.rgba.chunks_exact(4).map(|p| p[3]).collect();
                let engine_geometry = geometry(&ea, w);
                let reference_geometry = geometry(&ra, w);
                if let Some(samples) = std::env::var("PM_TEXT_REFERENCE_SAMPLES")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                {
                    let higher = support::text_reference::render_with_samples(
                        &raster.plan.scene,
                        pm_protocol::color::WorkingEncoding::Standard,
                        samples,
                    );
                    let high_alpha: Vec<f64> = higher.rgba.chunks_exact(4).map(|p| p[3]).collect();
                    let high_geometry = geometry(&high_alpha, w);
                    println!(
                        "case {index} geometry oracle {samples} cells: engine {:?}; selected reference{reference_samples} {:?}; reference{samples} {:?}; selected verdict {:?}; oracle verdict {:?}",
                        engine_geometry,
                        reference_geometry,
                        high_geometry,
                        same_geometry(&engine_geometry, &reference_geometry, 1e-3),
                        same_geometry(&engine_geometry, &high_geometry, 1e-3),
                    );
                }
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
                for (i, class) in ideal
                    .classes
                    .iter()
                    .enumerate()
                    .filter(|(_, class)| **class == Class::Interior)
                {
                    let actual = &engine[i * 4..][..4];
                    let witness = &today[i * 4..][..4];
                    let reference = &ideal.rgba[i * 4..][..4];
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
                        "case {index} interior residual pixel ({},{}): original {original_cv:.3} CV; outline {modeled_cv:.3} CV; alpha engine {:.6} witness {:.6} reference {:.6}",
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
                "referenceModel": if ideal.unreferenced == 0 { "outline" } else { "outline-with-original-bound-gap" },
                "referenceSamples": reference_samples,
                "geometry": geometry_report,
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
