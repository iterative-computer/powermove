//! Native-only corpus: no private capture or installed app is needed.
//! The production served-font provider, actual renderer, bounds, picks,
//! anchor edits, 2.5D planes and the compatibility raster, with CoreText
//! drawing every posed frame (rotation, scale, skew, blur, color and
//! opacity included) as the independent pixel reference.
mod support;

#[cfg(target_os = "macos")]
use pm_eval::text_geom::Selection;

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

    // The forbid itself: level 2 content and level 2 text reasons are
    // routing reasons no corpus project may produce (checked per project
    // below against the renderer's own issue list).
    assert!(support::routing_forbidden(&Feature::CompatLevel2));
    assert!(support::routing_forbidden(&Feature::Text(
        "animated text at compat level 2".into()
    )));
    assert!(!support::routing_forbidden(&Feature::Text(
        "text layers without configured fonts".into()
    )));
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../corpus/fixtures/text-level2.json")).unwrap();
    assert_eq!(cases.len(), 14, "every level 2 corpus case is exercised");
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
    {
        use pm_eval::text_geom::TextMeasure;
        let normal = pm_text::compat::TextContent::from_json(
            &json!({"text":"Default weight","font":"Avenir Next","size":32}),
        );
        let mut zero = normal.clone();
        zero.weight = Some(0.0);
        let expected = measure.level2(&normal, &[]).unwrap().layout;
        let actual = measure.level2(&zero, &[]).unwrap().layout;
        assert_eq!(
            actual, expected,
            "font resolution and layout must use the same default weight"
        );
    }
    let mut request = RenderRequest::sized(Size::new(640, 360), Precision::Float32);
    request.background = false;
    let assets = MapResolver::default();
    selection_contract(&mut measure, &mut renderer);
    let mut frames = 0;
    let mut routes = support::text::Level2Routes::new();
    let mut stats = Stats::default();
    let mut ssim_failures = Vec::new();
    let make_doc = |l: Value| {
        pm_model::Document::new(
            Some(
                pm_model::json::parse(&project(640, 360, 3.0, no_fill(), vec![l]).to_string())
                    .unwrap(),
            ),
            1,
        )
    };
    let render = |renderer: &mut Renderer, doc: &pm_model::Document, t: f64| -> Vec<f32> {
        let frame = renderer
            .render(doc, "Ptest-root", support::ticks(t), &request, &assets)
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        renderer.read_rgba(&frame).unwrap()
    };
    for case in &cases {
        let id = case["id"].as_str().unwrap();
        let mut d = case["d"].clone();
        for key in ["styles", "animators", "fontAnchorBounds"] {
            if let Some(v) = case.get(key) {
                d[key] = v.clone();
            }
        }
        let has = |key: &str| {
            case.get(key)
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
        };
        let (animated, styled) = (has("animators"), has("styles"));
        let mut l = layer("text", "text", d, 100.0, 80.0, 3.0);
        l["compat"] = json!(2);
        let document = make_doc(l.clone());
        // A capture of this document would be a level 1 picture: the
        // parity gates route it here (text_corpus, the evaluation corpus).
        assert!(routes.route(id, id, &document), "{id} routes to this gate");
        let resting_doc = {
            let mut r = l.clone();
            r["d"]["animators"] = json!([]);
            make_doc(r)
        };
        let plane_doc = {
            let mut p = l.clone();
            p["threeD"] = json!(true);
            make_doc(p)
        };
        let comp = Comp::resolve(&document, "Ptest-root").unwrap();
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let layer = &scope.layers()[0];
        let text: Vec<u16> = case["d"]["text"].as_str().unwrap().encode_utf16().collect();
        let mut selections = Vec::new();
        for t in case["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
        {
            let evaluated = scope
                .level2_text(layer, t, &mut measure, true, true)
                .unwrap();
            let resting = scope
                .level2_text(layer, t, &mut measure, false, true)
                .unwrap();
            // A character no face covers keeps its place and draws the
            // primary face's `.notdef`, as level 1 and today's engine do.
            let notdef: Vec<usize> = evaluated
                .layout
                .runs
                .iter()
                .flat_map(|r| r.glyphs.iter().zip(&r.sources))
                .filter(|(g, _)| g.id == 0)
                .map(|(_, s)| *s)
                .collect();
            if id == "text-level2/notdef" {
                assert_eq!(evaluated.layout.missing_glyphs, 1, "{id}");
                assert_eq!(notdef, [5], "{id}: the uncovered character is .notdef");
                assert!(
                    evaluated
                        .layout
                        .units
                        .characters
                        .iter()
                        .any(|u| u.source_start == 5 && u.text == "\u{0378}"),
                    "{id}: the uncovered character keeps its unit"
                );
            } else {
                assert_eq!(evaluated.layout.missing_glyphs, 0, "{id}");
                assert!(notdef.is_empty(), "{id}: {notdef:?}");
            }

            // Glyph/source map: one source per glyph, inside the text, and
            // no source split over two glyph runs (a split run restarting
            // its sources would repeat them).
            let mut owner: std::collections::HashMap<usize, usize> = Default::default();
            for (r, run) in evaluated.layout.runs.iter().enumerate() {
                assert_eq!(
                    run.sources.len(),
                    run.glyphs.len(),
                    "cluster/glyph map {id}"
                );
                for s in &run.sources {
                    assert!(*s < text.len(), "{id}: source {s}");
                    let first = *owner.entry(*s).or_insert(r);
                    assert_eq!(first, r, "{id} at {t}: source {s} in runs {first} and {r}");
                }
            }
            // Every visible character draws (ligature continuations aside).
            for unit in &evaluated.layout.units.characters {
                let ligated = id == "text-level2/ligature" && unit.text == "\u{0627}";
                assert_eq!(
                    owner.contains_key(&unit.source_start),
                    !ligated,
                    "{id}: character {:?} at {}",
                    unit.text,
                    unit.source_start
                );
            }

            // Bounds and picks follow today's animated-text selection: the
            // resting styled box, which never collapses while animators
            // hide every glyph; range styles also select the posed ink.
            let raster = scope.text_raster(layer, t, &mut measure).unwrap();
            let bounds = scope
                .bounds(layer, t, &mut measure, &mut NoHost)
                .unwrap()
                .unwrap();
            let sel = evaluated.selection;
            assert_eq!(
                [bounds.x0, bounds.y0, bounds.x1, bounds.y1],
                [sel.x0, sel.y0, sel.x1, sel.y1],
                "{id}: bounds are the frame's selection"
            );
            assert_eq!(raster.selection, sel, "{id}");
            assert!(sel.w > 1.0 && sel.h > 1.0, "{id} at {t}: selection {sel:?}");
            assert_close(
                &evaluated.resting,
                &resting.resting,
                1e-9,
                &format!("{id} resting"),
            );
            let expected = match (&evaluated.ink, styled && animated) {
                (Some(ink), true) => union(&resting.selection, ink),
                _ => resting.selection,
            };
            assert_close(&sel, &expected, 1e-9, &format!("{id} selection at {t}"));
            if !styled {
                selections.push(sel);
            }
            let (cx, cy) = (
                100.0 + (sel.x0 + sel.x1) / 2.0,
                80.0 + (sel.y0 + sel.y1) / 2.0,
            );
            let pick =
                |scope: &mut Scope<'_, '_>, measure: &mut CompatMeasure<_>, x: f64, y: f64| {
                    scope
                        .pick(x, y, t, &PickOptions::default(), measure, &mut NoHost)
                        .unwrap()
                        .is_some()
                };
            assert!(
                pick(&mut scope, &mut measure, cx, cy),
                "{id} at {t}: pick at the selection's center"
            );
            assert!(
                !pick(&mut scope, &mut measure, 100.0 + sel.x1 + 30.0, cy),
                "{id} at {t}: pick right of the selection misses"
            );

            // Pixels: the posed ink box encloses every visible pixel (AA and
            // the CoreText profile may expand ink by one pixel).
            let pixels = render(&mut renderer, &document, t);
            let visible = pixels.chunks_exact(4).any(|p| p[3] > 0.01);
            assert_eq!(
                visible,
                evaluated.ink.is_some(),
                "{id} at {t}: ink box vs pixels"
            );
            let hidden = id == "text-level2/stagger" && t == 0.0;
            assert_eq!(visible, !hidden, "{id} at {t}");
            if let Some(ink) = evaluated.ink {
                for (i, p) in pixels
                    .chunks_exact(4)
                    .enumerate()
                    .filter(|(_, p)| p[3] > 0.1)
                {
                    let x = (i % 640) as f64 - 100.0;
                    let y = (i / 640) as f64 - 80.0;
                    assert!(
                        x >= ink.x0 - 2.0
                            && x <= ink.x1 + 2.0
                            && y >= ink.y0 - 2.0
                            && y <= ink.y1 + 2.0,
                        "{id} ink ({x},{y}) outside {ink:?}, alpha {}",
                        p[3]
                    );
                }
            }
            let ssim = compare_coretext(&evaluated, &pixels, id, t);
            stats.ssim(id, &evaluated, ssim);
            if let Some(ink) = ssim.filter(|ink| *ink < 0.98) {
                // Every frame is measured before the gate fails.
                ssim_failures.push(format!("{id} at {t}: CoreText ink SSIM {ink:.4}"));
            }

            // The same layer on an unturned 2.5D plane (the local_text
            // path) draws what the flat layer draws.
            let plane = support::rgba8(&render(&mut renderer, &plane_doc, t));
            let flat = support::rgba8(&pixels);
            let (fb, pb) = (alpha_box(&flat), alpha_box(&plane));
            let mean = flat
                .chunks_exact(4)
                .zip(plane.chunks_exact(4))
                .map(|(a, b)| f64::from(a[3].abs_diff(b[3])))
                .sum::<f64>()
                / (640.0 * 360.0);
            assert!(
                boxes_within(fb, pb, 1) && mean <= 0.5,
                "{id} at {t}: plane {pb:?} vs flat {fb:?}, mean alpha difference {mean:.4} CV"
            );
            stats.plane = stats.plane.max(mean);

            // The compatibility raster (`PM.raster`) covers the posed ink.
            // A frame with nothing visible has no raster, as at level 1.
            let compat = renderer
                .text_raster(&document, "Ptest-root", "text", t, 1.5, Precision::Float32)
                .unwrap();
            assert_eq!(
                compat
                    .as_ref()
                    .is_some_and(|c| c.pixels.chunks_exact(4).any(|p| p[3] > 0.01)),
                visible,
                "{id} raster at {t}"
            );
            if let Some(compat) = &compat {
                assert!(compat.anchor.0.is_finite() && compat.anchor.1.is_finite());
            }
            if let Some(ink) = evaluated.ink {
                let r = evaluated.raster();
                assert!(
                    -r.anchor_x <= ink.x0 + 1e-9
                        && -r.anchor_y <= ink.y0 + 1e-9
                        && r.w - r.anchor_x >= ink.x1 - 1e-9
                        && r.h - r.anchor_y >= ink.y1 - 1e-9,
                    "{id}: raster {r:?} clips ink {ink:?}"
                );
            }

            let still = (id == "text-level2/range-leading" && t == 0.0)
                .then(|| render(&mut renderer, &resting_doc, t));
            case_checks(
                id,
                t,
                &mut scope,
                layer,
                &evaluated,
                &resting,
                &pixels,
                still.as_deref(),
            );
            if case.get("fontAnchorBounds").is_some() {
                // Typography anchoring places the resting styled text at
                // the anchor (0, 0) at every time, whatever the motion.
                let r = evaluated.resting;
                assert!(
                    r.x0.abs() < 1e-8 && r.y0.abs() < 1e-8,
                    "{id} at {t}: anchored {r:?}"
                );
                anchor_edit(id, &l, t, &evaluated, &mut measure);
                stats.anchor_edits += 1;
            }
            frames += 1;
        }
        if animated
            && !selections.is_empty()
            && !id.contains("styled")
            && id != "text-level2/range-leading"
        {
            // Without typography keyframes the selection never moves.
            for s in &selections {
                assert_close(s, &selections[0], 1e-9, &format!("{id} stable selection"));
            }
        }
        let issues = renderer.unsupported_with_media(&document, "Ptest-root", &assets);
        assert!(
            !issues
                .iter()
                .any(|u| support::routing_forbidden(&u.feature)),
            "{id} forbidden routing: {issues:?}"
        );
        assert!(issues.is_empty(), "{id}: {issues:?}");
        // Warm frame/query reuse must not rebuild its collection, register
        // the faces again, or reshape the same typography. Include the
        // mixed-weight/range and large emoji cases, not just plain Latin.
        scope
            .level2_text(layer, 0.75, &mut measure, true, true)
            .unwrap();
        let before = measure.level2_cache_stats();
        scope
            .level2_text(layer, 0.75, &mut measure, true, true)
            .unwrap();
        let cache = measure.level2_cache_stats();
        assert_eq!(
            cache.registrations, before.registrations,
            "{id}: face registrations reused"
        );
        assert_eq!(cache.layouts, before.layouts, "{id}: layout reused");
        assert!(
            cache.layout_hits > before.layout_hits,
            "{id}: warm layout hit"
        );
        assert!(cache.retained_bytes <= cache.limit_bytes, "{id}: {cache:?}");
        assert!(
            cache.face_bytes <= cache.face_limit_bytes
                && cache.face_entries <= cache.face_limit_entries,
            "{id}: {cache:?}"
        );
        assert!(
            cache.set_entries <= cache.set_limit_entries,
            "{id}: {cache:?}"
        );
        assert!(
            cache.layout_bytes <= cache.layout_limit_bytes
                && cache.glyph_bytes <= cache.glyph_limit_bytes,
            "{id}: {cache:?}"
        );
        println!("{id}: cache {cache:?}");
        println!("{id}: four native frames, bounds, picks, plane, raster, no forbidden routing");
    }
    assert_eq!(frames, 56);
    assert_eq!(routes.routed.len(), cases.len());
    assert!(routes.failures().is_empty(), "{:?}", routes.failures());
    level2_routing_contract(&make_doc);
    stats.report();
    assert!(
        ssim_failures.is_empty(),
        "below the 0.98 CoreText ink SSIM floor: {ssim_failures:#?}"
    );
    println!("level 2 semantics: {frames} native frames passed");
}

/// The parity gates' level 2 routing: level 1 text is compared with the
/// capture, level 2 text the semantic gate judges is routed, and level 2
/// text it does not judge fails rather than being skipped.
#[cfg(target_os = "macos")]
fn level2_routing_contract(make_doc: &dyn Fn(serde_json::Value) -> pm_model::Document) {
    use serde_json::json;
    use support::doc::layer;
    let text = |compat: Option<u32>| {
        let mut l = layer(
            "text",
            "text",
            json!({"text":"a","font":"Georgia","size":20}),
            0.0,
            0.0,
            3.0,
        );
        if let Some(c) = compat {
            l["compat"] = json!(c);
        }
        make_doc(l)
    };
    let mut routes = support::text::Level2Routes::new();
    assert!(!routes.route("text/plain", "text/plain", &text(None)));
    assert!(!routes.route("text/level-1", "text/level-1", &text(Some(1))));
    assert!(routes.failures().is_empty());
    assert!(routes.route(
        "text-level2/word-transform (text in isolation)",
        "text-level2/word-transform",
        &text(Some(2))
    ));
    assert!(routes.failures().is_empty());
    assert!(routes.route("text/unjudged", "text/unjudged", &text(Some(2))));
    assert_eq!(routes.failures().len(), 1);
    assert!(routes.failures()[0].starts_with("text/unjudged: "));
    assert_eq!(routes.routed.len(), 2);
    println!(
        "level 2 routing contract: level 1 compared, judged level 2 routed, unjudged level 2 fails"
    );
}

/// Independent interaction contract: authored paragraph boxes and the
/// minimum 3px padding remain selectable even where there is no glyph ink.
#[cfg(target_os = "macos")]
fn selection_contract(
    measure: &mut pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>,
    renderer: &mut pm_render::Renderer,
) {
    use pm_eval::bounds::{NoHost, PickOptions};
    use pm_eval::{Comp, Evaluator, Scope};
    use serde_json::json;
    use support::doc::{layer, no_fill, project};

    let mut cases = Vec::new();
    for (align, x0) in [("left", 0.0), ("center", -130.0), ("right", -260.0)] {
        for height in [0.0, 220.0] {
            cases.push((
                format!("paragraph {align} height {height}"),
                json!({"text":"one\ntwo", "font":"Georgia", "size":12,
                    "leading":1.25,"boxWidth":260,"boxHeight":height,"align":align}),
                Some([
                    x0 - 3.0,
                    -3.0,
                    x0 + 263.0,
                    if height > 0.0 { 223.0 } else { 33.0 },
                ]),
            ));
        }
    }
    cases.push((
        "small point text".into(),
        json!({"text":"Tiny", "font":"Georgia", "size":8}),
        None,
    ));
    for text in ["   ", "", "\t\u{2003}\n"] {
        cases.push((
            format!("blank {text:?}"),
            json!({"text":text,"font":"Georgia","size":8}),
            None,
        ));
    }
    for (name, d, expected) in cases {
        let mut l = layer("text", "text", d, 320.0, 80.0, 3.0);
        l["compat"] = json!(2);
        let doc = pm_model::Document::new(
            Some(
                pm_model::json::parse(&project(640, 360, 3.0, no_fill(), vec![l]).to_string())
                    .unwrap(),
            ),
            1,
        );
        let comp = Comp::resolve(&doc, "Ptest-root").unwrap();
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let l = &scope.layers()[0];
        let frame = scope.level2_text(l, 0.0, measure, true, true).unwrap();
        let s = frame.selection;
        if let Some(expected) = expected {
            for (actual, expected) in [s.x0, s.y0, s.x1, s.y1].into_iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{name}: {s:?}, expected {expected}"
                );
            }
        } else if name.starts_with("blank") {
            assert_eq!(
                [s.x0, s.x1],
                [-3.5, 3.5],
                "{name}: blank x +/-0.5 plus interaction padding"
            );
            assert!(
                s.h >= 8.0 + 6.0,
                "{name}: non-collapsed blank selection {s:?}"
            );
            assert!(
                s.h >= frame.layout.height + 6.0,
                "{name}: all blank lines stay interactive"
            );
            assert!(frame.ink.is_none(), "{name}: blank text has no painted ink");
        } else {
            let r = frame.resting;
            for actual in [r.x0 - s.x0, r.y0 - s.y0, s.x1 - r.x1, s.y1 - r.y1] {
                assert!(
                    (actual - 3.0).abs() < 1e-8,
                    "{name}: minimum interaction padding {s:?} vs {r:?}"
                );
            }
        }
        // Pick inside the padded/paragraph edge, outside the tight ink.
        let x = 320.0 + s.x1 - 0.25;
        let y = 80.0 + (s.y0 + s.y1) / 2.0;
        assert!(
            scope
                .pick(x, y, 0.0, &PickOptions::default(), measure, &mut NoHost)
                .unwrap()
                .is_some(),
            "{name}: edge pick"
        );
        assert!(
            scope
                .pick(
                    x + 1.0,
                    y,
                    0.0,
                    &PickOptions::default(),
                    measure,
                    &mut NoHost
                )
                .unwrap()
                .is_none(),
            "{name}: outside pick"
        );
        let b = scope.bounds(l, 0.0, measure, &mut NoHost).unwrap().unwrap();
        assert_eq!([b.x0, b.y0, b.x1, b.y1], [s.x0, s.y0, s.x1, s.y1], "{name}");
        assert!(
            renderer
                .unsupported_with_media(&doc, "Ptest-root", &pm_render::MapResolver::default())
                .is_empty(),
            "{name}: native capability"
        );
        println!("selection contract: {name} passed");
    }
}

#[cfg(target_os = "macos")]
#[allow(clippy::too_many_arguments)]
fn case_checks<'d>(
    id: &str,
    t: f64,
    scope: &mut pm_eval::Scope<'_, 'd>,
    layer: &'d pm_model::Node,
    evaluated: &pm_eval::text_level2::Frame,
    resting: &pm_eval::text_level2::Frame,
    pixels: &[f32],
    still: Option<&[f32]>,
) {
    let ink = evaluated.ink;
    let rest = resting.ink.expect("resting text has ink");
    match id {
        // Exact transform invariants: a full range translation moves every
        // ink edge by the authored offset at t = 0 (keyframed afterwards).
        "text-level2/range-position" | "text-level2/anchor" if t == 0.0 => {
            let (dx, dy) = if id.ends_with("anchor") {
                (50.0, 20.0)
            } else {
                (72.0, 24.0)
            };
            let ink = ink.unwrap();
            for (actual, expected) in [
                (ink.x0, rest.x0 + dx),
                (ink.y0, rest.y0 + dy),
                (ink.x1, rest.x1 + dx),
                (ink.y1, rest.y1 + dy),
            ] {
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "{id}: {actual} vs {expected}"
                );
            }
        }
        // Tracking pushes the rest of the line and the line stays aligned:
        // centered text grows evenly, right-aligned text keeps its right edge.
        "text-level2/center-tracking" => {
            let ink = ink.unwrap();
            let c = |s: &Selection| (s.x0 + s.x1) / 2.0;
            assert!(
                (c(&ink) - c(&rest)).abs() < 1e-6,
                "{id} at {t}: centers {ink:?} {rest:?}"
            );
            if t == 0.0 {
                // Six characters, 12 px after each of the first five.
                assert!(
                    (ink.x0 - (rest.x0 - 30.0)).abs() < 1e-6,
                    "{id}: {ink:?} {rest:?}"
                );
            }
        }
        "text-level2/right-tracking" => {
            let ink = ink.unwrap();
            assert!(
                (ink.x1 - rest.x1).abs() < 1e-6,
                "{id} at {t}: {ink:?} {rest:?}"
            );
            if t == 0.0 {
                assert!(
                    (ink.x0 - (rest.x0 - 32.0)).abs() < 1e-6,
                    "{id}: {ink:?} {rest:?}"
                );
            }
        }
        "text-level2/range-leading" => {
            // Range colors land on the characters the ranges cover. Bands
            // come from caret edges, not the glyph/source map under test.
            let band = |range: std::ops::Range<usize>| {
                let units: Vec<_> = evaluated
                    .layout
                    .units
                    .characters
                    .iter()
                    .filter(|u| range.contains(&u.source_start))
                    .collect();
                let x0 = units.iter().map(|u| u.x).fold(f64::INFINITY, f64::min);
                let x1 = units
                    .iter()
                    .map(|u| u.x + u.w)
                    .fold(f64::NEG_INFINITY, f64::max);
                (x0, x1)
            };
            let classify = |p: &[f32]| {
                let (r, g, b) = (p[0] / p[3], p[1] / p[3], p[2] / p[3]);
                if r > 0.85 && b < 0.3 && g < 0.6 {
                    Some("orange")
                } else if r < 0.5 && b > 0.85 {
                    Some("blue")
                } else {
                    None
                }
            };
            for (name, range) in [("orange", 6..11), ("blue", 12..17)] {
                let (x0, x1) = band(range);
                let mut n = 0;
                for (i, p) in pixels
                    .chunks_exact(4)
                    .enumerate()
                    .filter(|(_, p)| p[3] > 0.9)
                {
                    if classify(p) == Some(name) {
                        let x = (i % 640) as f64 - 100.0;
                        assert!(
                            x >= x0 - 2.0 && x <= x1 + 2.0,
                            "{id} at {t}: {name} at {x} outside {x0}..{x1}"
                        );
                        n += 1;
                    }
                }
                assert!(n > 50, "{id} at {t}: {n} {name} pixels");
            }
            if t == 0.0 {
                // The selector (characters 0-50%) lifts characters 0..=6 by
                // 24 px and leaves 8..=14: per character band, the topmost
                // ink row against the same frame without animators.
                let still = still.expect("the frame without animators");
                let top = |px: &[f32], x0: f64, x1: f64| {
                    let (a, b) = (
                        (100.0 + x0 + 2.0).ceil() as usize,
                        (100.0 + x1 - 2.0).floor() as usize,
                    );
                    (0..360).find(|y| (a..=b).any(|x| px[(y * 640 + x) * 4 + 3] > 0.5))
                };
                for unit in &evaluated.layout.units.characters {
                    if unit.w < 8.0 || unit.index == 7 {
                        continue;
                    }
                    let (Some(moved), Some(base)) = (
                        top(pixels, unit.x, unit.x + unit.w),
                        top(still, unit.x, unit.x + unit.w),
                    ) else {
                        continue;
                    };
                    let lift = base as f64 - moved as f64;
                    let expected = if unit.index <= 6 { 24.0 } else { 0.0 };
                    assert!(
                        (lift - expected).abs() <= 1.0,
                        "{id}: character {} {:?} lifted {lift}, expected {expected}",
                        unit.index,
                        unit.text
                    );
                }
            }
        }
        // A ligature's glyph takes the pose of its first grapheme (lam),
        // never its continuation's (alef), where the two differ.
        "text-level2/ligature" if t == 0.75 => {
            let d = scope.resolved_text_content(layer, t).unwrap();
            let state = scope
                .text_animation_state(layer, t, &evaluated.layout.units)
                .unwrap();
            let poses = scope.level2_glyphs(&d, &evaluated.layout.units, &state);
            let pose = |source| {
                poses
                    .iter()
                    .find(|p| p.unit.source_start == source)
                    .unwrap()
                    .matrix
            };
            let (lam, alef) = (pose(6), pose(7));
            assert!(
                (lam[0] - alef[0]).abs() > 1e-3,
                "the sample separates the two poses"
            );
            let draw = evaluated
                .draws
                .iter()
                .find(|draw| draw.run.sources.contains(&6))
                .expect("the lam-alef glyph draws");
            for k in 0..4 {
                assert!(
                    (draw.matrix[k] - lam[k]).abs() < 1e-12,
                    "{id}: {:?} vs lam {lam:?}",
                    draw.matrix
                );
            }
        }
        _ => {}
    }
}

/// Pan Behind on a font-anchored level 2 text layer: the edit starts from
/// the resting styled box (never the animated one), so after the anchor
/// moves and the recorded font bounds are rescaled, the resting text stays
/// exactly where it was, whatever the animators do at that time.
#[cfg(target_os = "macos")]
fn anchor_edit(
    id: &str,
    l: &serde_json::Value,
    t: f64,
    before: &pm_eval::text_level2::Frame,
    measure: &mut pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>,
) {
    use pm_edit::anchor::{self, Kind};
    use pm_edit::{Edit, IdSource};
    use support::doc::{no_fill, project, set};
    // The edit kernels run over the canonical form (every composition in
    // `comps`), as the document host holds it.
    let doc = |l: serde_json::Value| {
        let project = project(640, 360, 3.0, no_fill(), vec![l]).to_string();
        let canonical =
            pm_model::canonical::canonical_project(pm_model::json::parse(&project).unwrap());
        pm_model::Document::new(Some(canonical), 1)
    };
    let original = doc(l.clone());
    let mut edit = Edit::new(
        &original,
        "Ptest-root",
        "Ptest-root",
        None,
        IdSource::new("#", 1),
    )
    .unwrap();
    let reference = anchor::anchor_reference(&mut edit, 0, Kind::Font, t, measure)
        .unwrap()
        .expect("recorded font bounds");
    let r = before.resting;
    assert_eq!(
        [reference.x0, reference.y0, reference.x1, reference.y1],
        [r.x0, r.y0, r.x1, r.y1],
        "{id} at {t}: anchor edits start from the resting styled box"
    );
    let mut moved = l.clone();
    set(&mut moved, "anchor.x", serde_json::json!(30));
    set(&mut moved, "anchor.y", serde_json::json!(10));
    let moved = doc(moved);
    let mut edit = Edit::new(
        &moved,
        "Ptest-root",
        "Ptest-root",
        None,
        IdSource::new("#", 1),
    )
    .unwrap();
    anchor::anchor_moved(&mut edit, 0, Kind::Font, t, &reference).unwrap();
    let after = edit.document().clone();
    let comp = pm_eval::Comp::resolve(&after, "Ptest-root").unwrap();
    let mut ev = pm_eval::Evaluator::new();
    let mut scope = pm_eval::Scope::open(&mut ev, &comp);
    let layer = &scope.layers()[0];
    let frame = scope.level2_text(layer, t, measure, true, true).unwrap();
    assert_close(
        &frame.resting,
        &before.resting,
        1e-6,
        &format!("{id} at {t}: resting text after Pan Behind"),
    );
    match (frame.ink, before.ink) {
        (Some(a), Some(b)) => assert_close(
            &a,
            &b,
            1e-6,
            &format!("{id} at {t}: posed ink after Pan Behind"),
        ),
        (a, b) => assert_eq!(a.is_some(), b.is_some()),
    }
}

#[cfg(target_os = "macos")]
fn assert_close(a: &Selection, b: &Selection, eps: f64, what: &str) {
    let ok = [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)]
        .iter()
        .all(|(x, y)| (x - y).abs() <= eps);
    assert!(ok, "{what}: {a:?} vs {b:?}");
}

#[cfg(target_os = "macos")]
fn union(a: &Selection, b: &Selection) -> Selection {
    let (x0, y0, x1, y1) = (
        a.x0.min(b.x0),
        a.y0.min(b.y0),
        a.x1.max(b.x1),
        a.y1.max(b.y1),
    );
    Selection {
        x0,
        y0,
        x1,
        y1,
        w: x1 - x0,
        h: y1 - y0,
    }
}

#[cfg(target_os = "macos")]
fn alpha_box(px: &[u8]) -> Option<[usize; 4]> {
    let mut b: Option<[usize; 4]> = None;
    for (i, p) in px.chunks_exact(4).enumerate() {
        if p[3] > 8 {
            let (x, y) = (i % 640, i / 640);
            let v = b.get_or_insert([x, y, x, y]);
            *v = [v[0].min(x), v[1].min(y), v[2].max(x), v[3].max(y)];
        }
    }
    b
}

#[cfg(target_os = "macos")]
fn boxes_within(a: Option<[usize; 4]>, b: Option<[usize; 4]>, tol: usize) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.iter().zip(&b).all(|(x, y)| x.abs_diff(*y) <= tol),
        _ => false,
    }
}

/// Minimum CoreText SSIM per pose class (what the frame exercises).
#[cfg(target_os = "macos")]
#[derive(Default)]
struct Stats {
    classes: std::collections::BTreeMap<&'static str, (f64, usize)>,
    plane: f64,
    anchor_edits: usize,
}

#[cfg(target_os = "macos")]
impl Stats {
    fn ssim(&mut self, _id: &str, frame: &pm_eval::text_level2::Frame, ssim: Option<f64>) {
        let Some(ssim) = ssim else { return };
        let visible = frame.draws.iter().filter(|d| d.opacity > 0.0);
        let mut classes = Vec::new();
        for d in visible {
            let m = d.matrix;
            if m[..4] != [1.0, 0.0, 0.0, 1.0] {
                classes.push("linear pose (rotation/scale/skew)");
            } else if m[4] != 0.0 || m[5] != 0.0 {
                classes.push("translation");
            }
            if d.blur > 0.0 {
                classes.push("blur");
            }
            if d.opacity < 1.0 {
                classes.push("partial opacity");
            }
        }
        if classes.is_empty() {
            classes.push("resting");
        }
        classes.dedup();
        for c in classes {
            let e = self.classes.entry(c).or_insert((1.0, 0));
            e.0 = e.0.min(ssim);
            e.1 += 1;
        }
    }
    fn report(&self) {
        for (class, (min, n)) in &self.classes {
            println!("CoreText ink SSIM, {class}: minimum {min:.4} over {n} frames");
        }
        println!(
            "unturned plane: maximum mean alpha difference {:.4} CV",
            self.plane
        );
        println!(
            "anchor edits: {} frames keep the resting text in place",
            self.anchor_edits
        );
    }
}

/// Draws the frame's runs with CoreText through each run's pose, alpha and
/// color (blur from a Gaussian of the CoreText coverage, the `filter:
/// blur(σ)` the painter applies) and compares ink SSIM. `None` for a frame
/// with nothing visible.
#[cfg(target_os = "macos")]
fn compare_coretext(
    frame: &pm_eval::text_level2::Frame,
    ours: &[f32],
    id: &str,
    time: f64,
) -> Option<f64> {
    use pm_text::render::coretext::{CoreTextReference, RefRun};
    use read_fonts::TableProvider;
    let draws: Vec<_> = frame
        .draws
        .iter()
        .filter(|d| d.opacity.is_finite() && d.opacity > 0.0)
        .collect();
    if draws.is_empty() {
        return None;
    }
    let coordinates: Vec<Vec<(u32, f32)>> = draws
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
    let (ox, oy) = (frame.offset.x + 100.0, frame.offset.y + 80.0);
    let mut runs = Vec::new();
    let mut matrices = Vec::new();
    for (draw, coords) in draws.iter().zip(&coordinates) {
        assert!(!draw.run.synthetic_bold && draw.run.skew == 0.0);
        let m = draw.matrix;
        let mut color = pm_text::render::color::parse_or(
            &draw.color.to_js_string().to_string_lossy(),
            [1.0; 4],
        );
        color[3] *= draw.opacity;
        // A translation folds into the glyph positions (the exact integer
        // movement the earlier gate measured); other poses go through the
        // context transform.
        let translate = m[..4] == [1.0, 0.0, 0.0, 1.0];
        let (gx, mut gy) = if translate {
            (m[4] + ox, m[5] + oy)
        } else {
            (0.0, 0.0)
        };
        // Amendment 10: Apple Color Emoji placement follows Canvas2D at
        // both levels, whole pixels below where CoreText draws it.
        let face = pm_text::render::outline::FaceRef {
            data: draw.data.as_slice(),
            index: 0,
            id: draw.data.id(),
        };
        if translate && pm_text::render::bitmap::is_apple_color_emoji(face) {
            gy += pm_text::render::bitmap::apple_emoji_shift(draw.run.size, draw.run.size);
        }
        matrices.push(if translate {
            [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        } else {
            [m[0], m[1], m[2], m[3], m[4] + ox, m[5] + oy]
        });
        runs.push(RefRun {
            data: draw.data.as_slice(),
            index: 0,
            size: draw.run.size,
            coords,
            glyphs: draw
                .run
                .glyphs
                .iter()
                .map(|g| (g.id as u16, g.x + gx, g.y + gy))
                .collect(),
            color,
        });
    }
    let mut reference = CoreTextReference::new();
    let theirs = if draws.iter().any(|d| d.blur > 0.0) {
        // Each draw blurred on its own, then composited in order.
        let mut out = vec![0.0f32; 640 * 360 * 4];
        for (i, draw) in draws.iter().enumerate() {
            let mut layer = reference.render_posed(
                std::slice::from_ref(&runs[i]),
                &matrices[i..=i],
                640,
                360,
                true,
            );
            if draw.blur > 0.0 {
                gaussian(&mut layer, 640, 360, draw.blur);
            }
            for (o, s) in out.chunks_exact_mut(4).zip(layer.chunks_exact(4)) {
                let k = 1.0 - s[3];
                for c in 0..4 {
                    o[c] = s[c] + o[c] * k;
                }
            }
        }
        out
    } else {
        reference.render_posed(&runs, &matrices, 640, 360, true)
    };
    let to8 = |pixels: &[f32]| {
        pixels
            .iter()
            .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
            .collect::<Vec<_>>()
    };
    let ours = to8(ours);
    let theirs = to8(&theirs);
    let (ink, _, _) = pm_render::parity::text_ssim(&theirs, &ours, 640, 360);
    if let Some(dir) = std::env::var_os("PM_TEXT_LEVEL2_SHOTS") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let id = id.replace('/', "-");
        for (suffix, px) in [("native", &ours), ("coretext", &theirs)] {
            support::write_png(
                &dir.join(format!("{id}-{time}-{suffix}.png")),
                640,
                360,
                &support::text::png_rgba(px, true),
            );
        }
    }
    println!("{id} at {time}: CoreText ink SSIM {ink:.4}");
    Some(ink)
}

/// A separable Gaussian blur of standard deviation `sigma` px over
/// premultiplied RGBA (3σ kernel radius, edges clamped to transparent).
#[cfg(target_os = "macos")]
fn gaussian(px: &mut [f32], w: usize, h: usize, sigma: f64) {
    let radius = (3.0 * sigma).ceil() as isize;
    let kernel: Vec<f32> = (-radius..=radius)
        .map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp() as f32)
        .collect();
    let sum: f32 = kernel.iter().sum();
    let kernel: Vec<f32> = kernel.iter().map(|k| k / sum).collect();
    let pass = |src: &[f32],
                dst: &mut [f32],
                len: usize,
                stride: usize,
                lines: usize,
                line_stride: usize| {
        for line in 0..lines {
            for i in 0..len as isize {
                let mut acc = [0.0f32; 4];
                for (k, weight) in kernel.iter().enumerate() {
                    let j = i + k as isize - radius;
                    if j < 0 || j >= len as isize {
                        continue;
                    }
                    let at = line * line_stride + j as usize * stride;
                    for c in 0..4 {
                        acc[c] += src[at + c] * weight;
                    }
                }
                let at = line * line_stride + i as usize * stride;
                dst[at..at + 4].copy_from_slice(&acc);
            }
        }
    };
    let mut tmp = vec![0.0f32; px.len()];
    pass(px, &mut tmp, w, 4, h, w * 4);
    pass(&tmp, px, h, w * 4, w, 4);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
