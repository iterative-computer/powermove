//! Text and caption colors as today's canvas setters take them:
//!
//! - `hsl()` and `hwb()` draw the sRGB color Chromium stores;
//! - a string the setter rejects keeps the style that `save()` restored
//!   (black on a fresh canvas, the box fill under a caption box), never the
//!   previous glyph's or word's color;
//! - `hexToRgba` strings holding `NaN` are rejected the same way;
//! - CSS Color 4/5 colors (`oklch()`, `lab()`, `color()`, `color-mix()`,
//!   relative colors, `currentcolor`) draw (`pm_text::render::color4`; their
//!   pixels are gated on the closure capture in `text_raster`);
//! - a valid CSS color the engine does not reproduce (`light-dark()`, system
//!   colors not measured) refuses the layer, in `text_raster` and in
//!   `render`, and the refusal is not left behind for the next layer.
//!
//! Runs without the harness (the CoreText registry needs the main thread).

mod support;

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_render::{Feature, RenderError, Renderer};
    use serde_json::{Value, json};
    use support::doc::{ch, layer, no_fill, project};

    let mut r = Renderer::headless().expect("a Metal device");
    r.set_text_fonts(Some(Box::new(support::text::MainThreadFonts(
        pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new()),
        None,
    ))));
    let doc_of = |ty: &str, d: &Value| {
        let l = layer("l", ty, d.clone(), 960.0, 540.0, 10.0);
        let v = project(1920, 1080, 10.0, no_fill(), vec![l]);
        pm_model::Document::new(Some(pm_model::json::parse(&v.to_string()).unwrap()), 1)
    };
    let text = |color: &str| {
        json!({"text": "Color 123", "font": "SF Pro Display", "weight": 700, "size": 48,
            "color": color, "align": "left"})
    };
    let ranged = |base: &str, second: &str| {
        let style = |id: &str, start: u32, end: u32, color: &str| json!({"id": id, "start": start, "end": end, "p": {"color": ch(json!(color))}});
        json!({"text": "Ranged colors", "font": "SF Pro Display", "weight": 700, "size": 48,
            "color": base, "align": "left",
            "styles": [style("s1", 0, 3, "#FF0000"), style("s2", 3, 6, second)]})
    };
    let caption = |fill: &str, highlight: &str, box_color: &str, stroke: &str, boxed: bool| {
        json!({
            "cues": [{"id": "c1", "text": "Colors in a caption", "start": 0, "end": 3}],
            "style": {"preset": "pop", "font": "SF Pro Display", "weight": 900, "italic": false,
                "size": 64, "tracking": 0, "leading": 1.05, "fill": fill, "stroke": 6,
                "strokeColor": stroke, "box": boxed, "boxColor": box_color, "boxOpacity": 100,
                "boxPadding": 14, "boxRadius": 10, "placement": "middle", "align": "center",
                "offsetX": 0, "offsetY": 0, "safeMargin": 8, "maxWidth": 76, "maxLines": 2,
                "textCase": "none", "highlight": true, "highlightColor": highlight,
                "highlightScale": 100, "wordAnimation": "none"}
        })
    };
    let mut failures: Vec<String> = Vec::new();
    let raster = |r: &mut Renderer, ty: &str, d: &Value| {
        r.text_raster(
            &doc_of(ty, d),
            "Ptest-root",
            "l",
            1.05,
            1.0,
            Precision::Float32,
        )
    };

    // Equal scenes: the color a string draws is the color of `expected`.
    let same = [
        (
            "hsl() as hex",
            "text",
            text("hsl(0 100% 50%)"),
            text("#FF0000"),
        ),
        (
            "hsla() legacy syntax",
            "text",
            text("hsla(240, 100%, 50%, 0.5)"),
            text("rgba(0, 0, 255, 0.5)"),
        ),
        ("hwb()", "text", text("hwb(120 0% 0%)"), text("#00FF00")),
        (
            "rejected text color is black",
            "text",
            text("bogus"),
            text("#000000"),
        ),
        (
            "rejected glyph color is black, not the previous glyph's",
            "text",
            ranged("#00FF00", "bogus"),
            ranged("#00FF00", "#000000"),
        ),
        (
            "hsl() in a range style",
            "text",
            ranged("#00FF00", "hsl(240 100% 50%)"),
            ranged("#00FF00", "#0000FF"),
        ),
        (
            "rejected caption fill takes the box color",
            "captions",
            caption("bogus", "bogus", "#202020", "#000000", true),
            caption("#202020", "#202020", "#202020", "#000000", true),
        ),
        (
            "rejected caption fill without a box is black",
            "captions",
            caption("bogus", "bogus", "#202020", "#333333", false),
            caption("#000000", "#000000", "#202020", "#333333", false),
        ),
        (
            "rejected highlight color takes the box color, not the previous word's",
            "captions",
            caption("#FFFFFF", "bogus", "#202020", "#000000", true),
            caption("#FFFFFF", "#202020", "#202020", "#000000", true),
        ),
        (
            "a short box code (rgba with NaN) fills black",
            "captions",
            caption("#FFFFFF", "#FFD43B", "#fff", "#000000", true),
            caption("#FFFFFF", "#FFD43B", "#000000", "#000000", true),
        ),
        (
            "a short stroke code (rgba with NaN) strokes black",
            "captions",
            caption("#FFFFFF", "#FFD43B", "#202020", "#fff", true),
            caption("#FFFFFF", "#FFD43B", "#202020", "#000000", true),
        ),
    ];
    for (name, ty, d, expected) in &same {
        match (raster(&mut r, ty, d), raster(&mut r, ty, expected)) {
            (Ok(Some(a)), Ok(Some(b))) => {
                let equal = a.plan.key == b.plan.key && a.pixels == b.pixels;
                println!("{name:<72} {}", if equal { "equal" } else { "DIFFERS" });
                if !equal {
                    failures.push(format!("{name}: draws differently from the expected color"));
                }
            }
            (a, b) => failures.push(format!(
                "{name}: {:?} / {:?}",
                a.map(|r| r.is_some()),
                b.map(|r| r.is_some())
            )),
        }
    }
    // The cases above can fail: the colors they tell apart draw
    // differently (carrying the previous glyph's or word's color forward,
    // as the engine once did, would not be equal to the expected scene).
    let differ = [
        ("text", text("#FF0000"), text("#000000")),
        (
            "text",
            ranged("#00FF00", "#000000"),
            ranged("#00FF00", "#FF0000"),
        ),
        (
            "captions",
            caption("#FFFFFF", "#202020", "#202020", "#000000", true),
            caption("#FFFFFF", "#FFFFFF", "#202020", "#000000", true),
        ),
        (
            "captions",
            caption("#202020", "#202020", "#202020", "#000000", true),
            caption("#000000", "#000000", "#202020", "#000000", true),
        ),
        (
            "captions",
            caption("#FFFFFF", "#FFD43B", "#000000", "#000000", true),
            caption("#FFFFFF", "#FFD43B", "#FFFFFF", "#000000", true),
        ),
    ];
    for (i, (ty, a, b)) in differ.iter().enumerate() {
        let a = raster(&mut r, ty, a).unwrap().unwrap();
        let b = raster(&mut r, ty, b).unwrap().unwrap();
        if a.plan.key == b.plan.key {
            failures.push(format!("sanity case {i}: the colors draw identically"));
        }
    }

    // CSS Color 4/5 colors draw.
    for (name, ty, d) in [
        ("oklch() text", "text", text("oklch(62.8% 0.258 29.2)")),
        ("lab() text", "text", text("lab(50% 40 59.5)")),
        ("color() text", "text", text("color(display-p3 1 0 0)")),
        ("currentcolor text", "text", text("currentcolor")),
        ("oklch() range style", "text", ranged("#00FF00", "oklch(70% 0.1 200)")),
        (
            "color-mix() caption fill",
            "captions",
            caption("color-mix(in srgb, red, blue)", "#FFD43B", "#202020", "#000000", true),
        ),
    ] {
        match raster(&mut r, ty, &d) {
            Ok(Some(_)) => println!("{name:<72} drawn"),
            Ok(None) => failures.push(format!("{name}: text_raster drew nothing")),
            Err(e) => failures.push(format!("{name}: text_raster refused: {e}")),
        }
    }
    // Valid CSS colors the engine does not reproduce refuse the layer.
    let refused = [
        ("light-dark() text", "text", text("light-dark(red, blue)")),
        ("system color text", "text", text("ButtonFace")),
        (
            "light-dark() range style",
            "text",
            ranged("#00FF00", "light-dark(#000, #fff)"),
        ),
        (
            "system color caption fill",
            "captions",
            caption("Highlight", "#FFD43B", "#202020", "#000000", true),
        ),
    ];
    let plain = text("#F2F2F2");
    let request = pm_render::RenderRequest::sized(
        pm_protocol::common::Size::new(640, 360),
        Precision::Float16,
    );
    let resolver = pm_render::MapResolver::default();
    for (name, ty, d) in &refused {
        let is_color_refusal = |e: &RenderError| match e {
            RenderError::Unsupported(u) => u.iter().any(|u| {
                matches!(&u.feature, Feature::Text(w) if w.starts_with("text color"))
                    && u.layer.as_deref() == Some("l")
            }),
            _ => false,
        };
        match raster(&mut r, ty, d) {
            Err(e) if is_color_refusal(&e) => println!("{name:<72} refused ({e})"),
            Err(e) => failures.push(format!("{name}: text_raster: unexpected error {e}")),
            Ok(_) => failures.push(format!("{name}: text_raster drew it")),
        }
        // Nothing is left for the next layer.
        if let Err(e) = raster(&mut r, "text", &plain) {
            failures.push(format!("{name}: the next text_raster failed: {e}"));
        }
        match r.render(
            &doc_of(ty, d),
            "Ptest-root",
            support::ticks(1.05),
            &request,
            &resolver,
        ) {
            Err(e) if is_color_refusal(&e) => {}
            Err(e) => failures.push(format!("{name}: render: unexpected error {e}")),
            Ok(_) => failures.push(format!("{name}: render drew it")),
        }
        if let Err(e) = r.render(
            &doc_of("text", &plain),
            "Ptest-root",
            support::ticks(1.05),
            &request,
            &resolver,
        ) {
            failures.push(format!("{name}: the next render failed: {e}"));
        }
    }
    for f in &failures {
        println!("FAILED {f}");
    }
    assert!(failures.is_empty());
}

#[cfg(not(target_os = "macos"))]
fn main() {}
