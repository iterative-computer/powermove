//! Level 1 ink of a color glyph is what the painter draws (TEXT-INK-1):
//! the Apple Color Emoji strike's nonzero texels at CoreText's full-em size
//! with the authored downward shift, plus the reconstruction's reach
//! (`pm_text::render::bitmap::ink_bounds`, proven against painted pixels
//! there), not the glyph's `glyf` box. Runs on the main thread: the
//! CoreText registry uses AppKit's font manager.

#[cfg(target_os = "macos")]
fn main() {
    use pm_eval::text_geom::TextMeasure;
    use pm_eval::text_measure::CompatMeasure;
    use pm_text::compat::TextContent;
    use pm_text::font::FontResolver;
    use pm_text::registry::CoreTextRegistry;

    let mut measure = CompatMeasure::new(CoreTextRegistry::new());
    let content = |text: &str, size: f64| TextContent {
        text: text.into(),
        font: Some("SF Pro Display".into()),
        weight: Some(500.0),
        size: Some(size),
        tracking: Some(0.0),
        leading: Some(1.15),
        ..Default::default()
    };
    let ink = |measure: &mut CompatMeasure<CoreTextRegistry>, text: &str, size: f64| {
        let [line] = &measure.measure_lines(&content(text, size))[..] else {
            panic!("one line");
        };
        line.ink
    };

    // G-T1/G-T5 case 519 ("sdf-scripts-0024"): the wave's top row reaches
    // 0.125 px above its strike (bilinear enlargement by 1.25) and the art
    // ends inside its strike at the bottom and right. Rounded out at the
    // 164 px baseline this is the app's nonzero-alpha box [15, -12, 694,
    // 187]. Canvas reported y -177..26 and x1 702 (Skia's round-out plus a
    // 1 px outset of CoreText's metric rectangle), which nothing draws.
    assert_eq!(
        ink(
            &mut measure,
            "Hi \u{1f44b}\u{1f3fd} \u{2764}\u{fe0f}",
            200.0
        ),
        [15.72265625, -175.125, 693.875, 22.625],
        "case 519 ink is the drawn pixels"
    );

    // A lone emoji at several sizes, including the enlarged full-em draw
    // below 24 px, measures exactly the painter's drawn area.
    let emoji = {
        let r = measure.compat().resolver_mut();
        let desc = pm_text::font::FontDesc {
            weight: 400.0,
            italic: false,
            size: 40.0,
        };
        let face = r
            .family("Apple Color Emoji", &desc)
            .expect("Apple Color Emoji is installed");
        let info = r.face(face).clone();
        let glyph = skrifa::MetadataProvider::charmap(
            &pm_text::render::outline::FaceRef::of(&info).font().unwrap(),
        )
        .map('\u{1f44b}')
        .unwrap()
        .to_u32();
        (face, info, glyph)
    };
    for size in [12.0f32, 23.0, 40.0, 96.0, 200.0] {
        let em = measure
            .compat()
            .resolver_mut()
            .em_advance(emoji.0, size)
            .unwrap_or(size);
        let drawn = pm_text::render::bitmap::ink_bounds(
            pm_text::render::outline::FaceRef::of(&emoji.1),
            emoji.2,
            size,
            em,
        )
        .unwrap()
        .unwrap();
        let expected = [drawn.x0, drawn.y0, drawn.x1, drawn.y1].map(|v| f64::from(v as f32));
        assert_eq!(
            ink(&mut measure, "\u{1f44b}", f64::from(size)),
            expected,
            "size {size} (drawn at {em})"
        );
    }

    saved_anchors_keep_todays_placement(&mut measure);
    println!("ok color glyph ink");
}

/// Typography anchoring measures text as Canvas did, so a layer saved with
/// `fontAnchorBounds` renders exactly where today's app puts it; only the
/// selection shows the drawn ink. Canvas's emoji box is Skia's glyph
/// metrics from CoreText's rectangle (round out, 1 px outset, through the
/// synthetic-italic skew; the canonical 64 px strike once a text-matrix
/// column exceeds 256 px). The expected boxes are today's app's
/// `measureText` `actualBoundingBox*` (Electron on macOS 27).
#[cfg(target_os = "macos")]
fn saved_anchors_keep_todays_placement(
    measure: &mut pm_eval::text_measure::CompatMeasure<pm_text::registry::CoreTextRegistry>,
) {
    use pm_eval::text_geom::TextMeasure;
    use pm_eval::{Comp, Evaluator, Scope};
    use pm_model::{Document, json::parse};

    let content = |text: &str, size: f64, italic: bool| pm_text::compat::TextContent {
        text: text.into(),
        font: Some("SF Pro Display".into()),
        weight: Some(500.0),
        size: Some(size),
        italic: Some(italic),
        tracking: Some(0.0),
        leading: Some(1.15),
        ..Default::default()
    };
    let wave = "\u{1f44b}";
    let case_519 = "Hi \u{1f44b}\u{1f3fd} \u{2764}\u{fe0f}";
    for (text, size, italic, canvas) in [
        (wave, 12.0, false, [-1.0, -13.0, 16.0, 4.0]),
        (wave, 23.0, false, [-1.0, -22.0, 25.0, 5.0]),
        (wave, 100.0, false, [-1.0, -89.0, 102.0, 14.0]),
        (wave, 200.0, false, [-1.0, -177.0, 202.0, 26.0]),
        (wave, 255.0, false, [-1.0, -225.0, 257.0, 33.0]),
        (
            wave,
            257.0,
            false,
            [-4.015625, -232.90625, 265.03125, 36.140625],
        ),
        (wave, 300.0, false, [-4.6875, -271.875, 309.375, 42.1875]),
        (wave, 100.0, true, [-5.0, -89.0, 123.0, 14.0]),
        (wave, 250.0, true, [-11.71875, -226.5625, 312.5, 35.15625]),
        (case_519, 200.0, false, [15.72265625, -177.0, 702.0, 26.0]),
        (case_519, 120.0, false, [9.43359375, -107.0, 422.0, 16.0]),
    ] {
        let [line] = &measure.measure_lines(&content(text, size, italic))[..] else {
            panic!("one line");
        };
        assert_eq!(
            line.anchor_ink, canvas,
            "Canvas box of {text:?} at {size} px, italic {italic}"
        );
        assert_ne!(
            line.ink, canvas,
            "selection shows the drawn ink of {text:?} at {size} px"
        );
    }

    // G-T5 case 519 saved in today's app with typography anchoring: the
    // recorded bounds are today's 200 px selection. Unchanged, the layer
    // stays put; resized to 120 px it moves exactly as today's arithmetic
    // (`fontAnchorOffset`) moves it from today's 120 px selection (its
    // Canvas box above through `textGeometry`: pad 96, baseline 194.4,
    // inset 6.6).
    let recorded = [4.72265625, -24.0, 713.0, 201.0];
    for (size, today) in [
        (200.0, recorded),
        (120.0, [2.83359375, -15.2, 428.6, 121.0]),
    ] {
        let layer = format!(
            r#"{{"id":"t","type":"text","from":0,"dur":10,"on":true,
            "p":{{"position.x":{{"v":960,"kf":[],"expr":null}},"position.y":{{"v":540,"kf":[],"expr":null}},
            "anchor.x":{{"v":37,"kf":[],"expr":null}},"anchor.y":{{"v":-15,"kf":[],"expr":null}}}},
            "d":{{"text":"{case_519}","font":"SF Pro Display","weight":500,
            "size":{size},"tracking":0,"leading":1.15,
            "fontAnchorBounds":{{"x0":4.72265625,"y0":-24,"x1":713,"y1":201}}}}}}"#
        );
        let doc = Document::new(
            Some(
                parse(&format!(
                    r#"{{"id":"P","compId":"C","w":1920,"h":1080,"fps":30,"dur":10,"layers":[{layer}],"comps":{{}},"params":{{}}}}"#
                ))
                .unwrap(),
            ),
            1,
        );
        let comp = Comp::resolve(&doc, "C").unwrap();
        let mut ev = Evaluator::new();
        let mut scope = Scope::open(&mut ev, &comp);
        let l = &scope.layers()[0];
        let free = scope
            .text_raster_with(l, 0.0, measure, true, false)
            .unwrap();
        let placed = scope.text_raster(l, 0.0, measure).unwrap();
        let b = free.anchor_box;
        let measured = [b.x0, b.y0, b.x1, b.y1];
        assert!(
            measured
                .iter()
                .zip(today)
                .all(|(m, t)| (m - t).abs() < 1e-9),
            "today's selection at {size} px: {measured:?}"
        );
        assert_ne!(
            free.selection, free.anchor_box,
            "the selection is the drawn ink"
        );
        let offset = |r0: f64, r1: f64, n0: f64, n1: f64, a: f64| {
            a - (n0 + (a - r0) / (r1 - r0) * (n1 - n0))
        };
        let (dx, dy) = (
            offset(recorded[0], recorded[2], measured[0], measured[2], 37.0),
            offset(recorded[1], recorded[3], measured[1], measured[3], -15.0),
        );
        if size == 200.0 {
            assert_eq!((dx, dy), (0.0, 0.0));
        }
        assert_eq!(
            (placed.anchor_x, placed.anchor_y),
            (free.anchor_x - dx, free.anchor_y - dy),
            "the saved anchor at {size} px renders where today's app renders it"
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
