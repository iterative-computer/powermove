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
    println!("ok color glyph ink");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
