//! Native-only corpus: no private capture or installed app is needed.
//! The production served-font provider, actual renderer, bounds and picks.
mod support;

#[cfg(target_os = "macos")]
fn main() {
    use pm_eval::bounds::{NoHost, PickOptions};
    use pm_eval::text_measure::CompatMeasure;
    use pm_eval::{Comp, Evaluator, Scope};
    use pm_protocol::{color::Precision, common::Size};
    use pm_render::{Feature, MapResolver, RenderRequest, Renderer};
    use pm_text::registry::CoreTextRegistry;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use support::doc::{layer, no_fill, project};

    assert!(support::routing_forbidden(&Feature::CompatLevel2));
    assert!(support::routing_forbidden(&Feature::Text(
        "animated text at compat level 2".into()
    )));
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../corpus/fixtures/text-level2.json")).unwrap();
    assert_eq!(cases.len(), 8, "every level 2 corpus case is exercised");
    let server = Arc::new(pm_text::served::DirectServer::new(
        CoreTextRegistry::new(),
        |r| r.generation(),
    ));
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_text_fonts(Some(Box::new(pm_render::text::served::ServedFonts::new(
        server,
        Arc::new(pm_render::text::served::NoSeeds),
    ))));
    let mut measure = CompatMeasure::new(CoreTextRegistry::new());
    let mut request = RenderRequest::sized(Size::new(640, 360), Precision::Float32);
    request.background = false;
    let assets = MapResolver::default();
    let mut frames = 0;
    for case in &cases {
        let mut d = case["d"].clone();
        for key in ["styles", "animators", "fontAnchorBounds"] {
            if let Some(v) = case.get(key) {
                d[key] = v.clone();
            }
        }
        let mut l = layer("text", "text", d, 100.0, 80.0, 3.0);
        l["compat"] = json!(2);
        let make_doc = |l: Value| {
            pm_model::Document::new(
                Some(
                    pm_model::json::parse(&project(640, 360, 3.0, no_fill(), vec![l]).to_string())
                        .unwrap(),
                ),
                1,
            )
        };
        let document = make_doc(l.clone());
        let comp = Comp::resolve(&document, "Ptest-root").unwrap();
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let layer = &scope.layers()[0];
        for t in case["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
        {
            let evaluated = scope
                .level2_text(layer, t, &mut measure, true, true)
                .unwrap();
            assert_eq!(evaluated.layout.missing_glyphs, 0, "{}", case["id"]);
            for run in &evaluated.layout.runs {
                assert_eq!(
                    run.sources.len(),
                    run.glyphs.len(),
                    "cluster/glyph map {}",
                    case["id"]
                );
                assert!(
                    run.sources
                        .iter()
                        .all(|s| *s < case["d"]["text"].as_str().unwrap().encode_utf16().count())
                );
            }
            let raster = scope.text_raster(layer, t, &mut measure).unwrap();
            let bounds = scope
                .bounds(layer, t, &mut measure, &mut NoHost)
                .unwrap()
                .unwrap();
            assert_eq!(
                [bounds.x0, bounds.y0, bounds.x1, bounds.y1],
                [
                    raster.selection.x0,
                    raster.selection.y0,
                    raster.selection.x1,
                    raster.selection.y1
                ]
            );
            let frame = renderer
                .render(
                    &document,
                    "Ptest-root",
                    support::ticks(t),
                    &request,
                    &assets,
                )
                .unwrap();
            assert!(
                frame.failures.is_empty(),
                "{}: {:?}",
                case["id"],
                frame.failures
            );
            let pixels = renderer.read_rgba(&frame).unwrap();
            if [
                "text-level2/range-position",
                "text-level2/stagger",
                "text-level2/styled-wrap",
                "text-level2/anchor",
            ]
            .contains(&case["id"].as_str().unwrap())
            {
                compare_coretext(&evaluated, &pixels, case["id"].as_str().unwrap(), t);
            }
            let visible = pixels.chunks_exact(4).any(|p| p[3] > 0.01);
            let hidden = case["id"] == "text-level2/stagger" && t == 0.0;
            assert_eq!(visible, !hidden, "{} at {t}", case["id"]);
            if visible {
                // Layer-space bounds must enclose every visible output pixel
                // (AA and the CoreText profile may expand ink by one pixel).
                for (i, p) in pixels
                    .chunks_exact(4)
                    .enumerate()
                    .filter(|(_, p)| p[3] > 0.1)
                {
                    let x = (i % 640) as f64 - 100.0;
                    let y = (i / 640) as f64 - 80.0;
                    assert!(
                        x >= bounds.x0 - 2.0
                            && x <= bounds.x1 + 2.0
                            && y >= bounds.y0 - 2.0
                            && y <= bounds.y1 + 2.0,
                        "{} ink ({x},{y}) outside {:?}, alpha {}",
                        case["id"],
                        bounds,
                        p[3]
                    );
                }
                let hit = scope
                    .pick(
                        100.0 + (bounds.x0 + bounds.x1) / 2.0,
                        80.0 + (bounds.y0 + bounds.y1) / 2.0,
                        t,
                        &PickOptions::default(),
                        &mut measure,
                        &mut NoHost,
                    )
                    .unwrap();
                assert!(
                    hit.is_some(),
                    "{} pick must follow animated bounds",
                    case["id"]
                );
            }
            frames += 1;
        }
        let issues = renderer.unsupported_with_media(&document, "Ptest-root", &assets);
        assert!(
            !issues
                .iter()
                .any(|u| support::routing_forbidden(&u.feature)),
            "{} forbidden routing: {issues:?}",
            case["id"]
        );
        assert!(issues.is_empty(), "{}: {issues:?}", case["id"]);
        let raster = renderer
            .text_raster(
                &document,
                "Ptest-root",
                "text",
                0.75,
                1.5,
                Precision::Float32,
            )
            .unwrap()
            .expect("level 2 compatibility raster");
        assert!(raster.pixels.chunks_exact(4).any(|p| p[3] > 0.01));
        assert!(raster.anchor.0.is_finite() && raster.anchor.1.is_finite());

        // Exact independent transform invariant. A full range translation
        // moves every ink edge and pick by the authored offset.
        if case["id"] == "text-level2/range-position" {
            let moved = scope.text_raster(layer, 0.0, &mut measure).unwrap();
            l["d"]["animators"] = json!([]);
            let plain = make_doc(l.clone());
            let plain_comp = Comp::resolve(&plain, "Ptest-root").unwrap();
            let mut plain_ev = Evaluator::new();
            let mut plain_scope = Scope::open(&mut plain_ev, &plain_comp);
            let plain_layer = &plain_scope.layers()[0];
            let resting = plain_scope
                .text_raster(plain_layer, 0.0, &mut measure)
                .unwrap();
            for (actual, expected) in [
                (moved.selection.x0, resting.selection.x0 + 72.0),
                (moved.selection.y0, resting.selection.y0 + 24.0),
                (moved.selection.x1, resting.selection.x1 + 72.0),
                (moved.selection.y1, resting.selection.y1 + 24.0),
            ] {
                assert!((actual - expected).abs() < 1e-8);
            }
            support::doc::set(&mut l, "position.x", json!(172));
            support::doc::set(&mut l, "position.y", json!(104));
            let translated = make_doc(l.clone());
            let pixels = renderer
                .render(
                    &translated,
                    "Ptest-root",
                    support::ticks(0.0),
                    &request,
                    &assets,
                )
                .and_then(|f| renderer.read_rgba(&f))
                .unwrap();
            let moved_pixels = renderer
                .render(
                    &document,
                    "Ptest-root",
                    support::ticks(0.0),
                    &request,
                    &assets,
                )
                .and_then(|f| renderer.read_rgba(&f))
                .unwrap();
            let mut max = 0.0f32;
            for (a, b) in pixels.iter().zip(&moved_pixels) {
                max = max.max((a - b).abs());
            }
            assert!(max < 1e-6, "integer translation changed ink: {max}");
        }
        if case["id"] == "text-level2/anchor" {
            let resting = scope
                .level2_text(layer, 0.0, &mut measure, false, true)
                .unwrap()
                .selection;
            assert!(
                resting.x0.abs() < 1e-8 && resting.y0.abs() < 1e-8,
                "typographic anchor origin"
            );
            let moving = scope
                .level2_text(layer, 0.0, &mut measure, true, true)
                .unwrap()
                .selection;
            assert!((moving.x0 - resting.x0 - 50.0).abs() < 1e-8);
            assert!((moving.y0 - resting.y0 - 20.0).abs() < 1e-8);
        }
        println!(
            "{}: four native frames, bounds, picks, no forbidden routing",
            case["id"]
        );
    }
    assert_eq!(frames, 32);
    println!("level 2 semantics: {frames} native frames passed");
}

#[cfg(target_os = "macos")]
fn compare_coretext(frame: &pm_eval::text_level2::Frame, ours: &[f32], id: &str, time: f64) {
    use pm_text::render::coretext::{CoreTextReference, RefRun};
    use read_fonts::TableProvider;
    let coordinates: Vec<Vec<(u32, f32)>> = frame
        .draws
        .iter()
        .map(|draw| {
            let font = read_fonts::FontRef::new(draw.data.as_slice()).unwrap();
            font.fvar()
                .ok()
                .and_then(|f| f.axes().ok())
                .map(|axes| {
                    axes.iter()
                        .zip(&draw.run.coords)
                        .map(|(axis, coord)| {
                            let n = f32::from(*coord) / 16384.0;
                            let default = axis.default_value().to_f32();
                            let value = default
                                + n * if n < 0.0 {
                                    default - axis.min_value().to_f32()
                                } else {
                                    axis.max_value().to_f32() - default
                                };
                            (u32::from_be_bytes(axis.axis_tag().into_bytes()), value)
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    let runs: Vec<_> = frame
        .draws
        .iter()
        .zip(&coordinates)
        .map(|(draw, coords)| {
            assert!(!draw.run.synthetic_bold && draw.run.skew == 0.0);
            let m = draw.matrix;
            assert_eq!(m[..4], [1.0, 0.0, 0.0, 1.0]);
            let mut color = pm_text::render::color::parse_or(
                &draw.color.to_js_string().to_string_lossy(),
                [1.0; 4],
            );
            color[3] *= draw.opacity;
            RefRun {
                data: draw.data.as_slice(),
                index: 0,
                size: draw.run.size,
                coords,
                glyphs: draw
                    .run
                    .glyphs
                    .iter()
                    .map(|g| {
                        (
                            g.id as u16,
                            g.x + m[4] + frame.offset.x + 100.0,
                            g.y + m[5] + frame.offset.y + 80.0,
                        )
                    })
                    .collect(),
                color,
            }
        })
        .collect();
    let theirs = CoreTextReference::new().render(&runs, 640, 360, true);
    let to8 = |pixels: &[f32]| {
        pixels
            .iter()
            .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect::<Vec<_>>()
    };
    let ours = to8(ours);
    let theirs = to8(&theirs);
    let (ink, _, _) = pm_render::parity::text_ssim(&theirs, &ours, 640, 360);
    assert!(ink >= 0.98, "{id} at {time}: CoreText ink SSIM {ink}");
    if let Some(dir) = std::env::var_os("PM_TEXT_LEVEL2_SHOTS") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let id = id.replace('/', "-");
        for (suffix, px) in [("native", ours), ("coretext", theirs)] {
            support::write_png(
                &dir.join(format!("{id}-{time}-{suffix}.png")),
                640,
                360,
                &support::text::png_rgba(&px, true),
            );
        }
    }
    println!("{id} at {time}: CoreText ink SSIM {ink:.4}");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
