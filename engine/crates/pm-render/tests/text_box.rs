//! A new text layer's box holds what it draws: the box the gizmo frames
//! (the query engine's `bounds`, fed whole font files as the native app's
//! text host feeds it) matches the drawn ink, for the default font and for a
//! family whose weights share one collection file (Helvetica 600 and 700 are
//! drawn Bold, from Helvetica.ttc).
//!
//! Runs without the harness (the CoreText registry needs the main thread).

mod support;

/// The layer's box (`bounds`) as the app's kernel answers it, in the
/// layer's own pixels about its anchor point: `[x0, y0, x1, y1]`.
#[cfg(target_os = "macos")]
fn app_box(registry: &mut pm_text::registry::CoreTextRegistry, project: &str) -> [f64; 4] {
    use pm_model::{Value, json::parse};
    let mut e = pm_wasm::Engine::new();
    e.resync(project.as_bytes(), 1).unwrap();
    for _ in 0..8 {
        let reply = e.query(br#"{"op":"bounds","comp":"Ptest-root","layer":0,"t":0}"#);
        let r = parse(&reply.text).unwrap();
        if let Some(ok) = r.get("ok") {
            let n = |k: &str| ok.get(k).and_then(Value::as_f64).unwrap();
            return [n("x0"), n("y0"), n("x1"), n("y1")];
        }
        if r.get("ink").is_some() {
            e.set_font_outlines(true);
            continue;
        }
        assert!(r.get("fonts").is_some(), "{}", reply.text);
        // `text_host::fill`: the misses answered by the registry, then each
        // face named, as its whole font file.
        let report: serde_json::Value = serde_json::from_str(&e.font_report()).unwrap();
        let misses: Vec<pm_text::answers::Query> =
            serde_json::from_value(report["misses"].clone()).unwrap();
        let mut wanted: Vec<(String, u32)> =
            serde_json::from_value(report["pending"].clone()).unwrap();
        let answers = registry.answer(&misses).unwrap();
        e.add_font_answers(serde_json::to_string(&answers).unwrap().as_bytes())
            .unwrap();
        wanted.extend(answers.faces.iter().map(|f| (f.blob.clone(), f.index)));
        for (blob, index) in wanted {
            e.add_whole_font_bytes(&blob, index, registry.blob(&blob).unwrap());
        }
    }
    panic!("the query engine never measured the layer");
}

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::{color::Precision, common::Size};
    use pm_render::{MapResolver, RenderRequest, Renderer};
    use support::doc::{layer, project, solid_fill};
    const W: usize = 1920;
    const H: usize = 1080;
    let mut renderer = Renderer::headless().expect("a Metal device");
    renderer.set_text_fonts(Some(Box::new(support::text::MainThreadFonts(
        pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new()),
        None,
    ))));
    let mut registry = pm_text::registry::CoreTextRegistry::new();
    let mut failures = vec![];
    for (font, weight) in [
        // New text's font, drawn in its fallback on a Mac without SF Pro.
        ("SF Pro Display", 600.),
        ("Helvetica", 600.),
        ("Helvetica", 700.),
        ("Helvetica", 400.),
        ("Helvetica Neue", 600.),
    ] {
        let d = serde_json::json!({
            "text": "Launch day", "font": font, "weight": weight, "size": 128,
            "tracking": -2, "leading": 1.1, "color": "#FFFFFF", "align": "center",
            "italic": false, "boxWidth": 0, "boxHeight": 0
        });
        let v = project(
            W as u32,
            H as u32,
            1.0,
            solid_fill("#000000"),
            vec![layer("t", "text", d, 960.0, 540.0, 1.0)],
        );
        let doc = pm_model::Document::new(Some(pm_model::json::parse(&v.to_string()).unwrap()), 1);
        let frame = renderer
            .render(
                &doc,
                "Ptest-root",
                support::ticks(0.0),
                &RenderRequest::sized(Size::new(W as u32, H as u32), Precision::Float32),
                &MapResolver::default(),
            )
            .expect("a text frame");
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let px = renderer.read_rgba(&frame).unwrap();
        // The drawn ink: every pixel lit above a tenth.
        let (mut x0, mut x1, mut y0, mut y1) = (W, 0, H, 0);
        for y in 0..H {
            for x in 0..W {
                if f64::from(px[(y * W + x) * 4]) > 0.1 {
                    (x0, x1, y0, y1) = (x0.min(x), x1.max(x + 1), y0.min(y), y1.max(y + 1));
                }
            }
        }
        // The box placed as the gizmo places it (the layer's world matrix:
        // its anchor point at its position, scale 100).
        let [bx0, by0, bx1, by1] = app_box(&mut registry, &v.to_string());
        let ((bx0, by0), (bx1, by1)) = ((960.0 + bx0, 540.0 + by0), (960.0 + bx1, 540.0 + by1));
        println!(
            "{font} {weight}: ink x {x0}..{x1} y {y0}..{y1}; box x {bx0:.1}..{bx1:.1} y {by0:.1}..{by1:.1}"
        );
        // The ink inside the box, to two pixels (antialiasing, the threshold).
        let out = [
            bx0 - x0 as f64,
            x1 as f64 - bx1,
            by0 - y0 as f64,
            y1 as f64 - by1,
        ];
        if out.iter().any(|o| *o > 2.0) {
            failures.push(format!(
                "{font} {weight}: the drawn ink leaves the box by {out:.1?} (left, right, top, bottom)"
            ));
        }
    }
    for f in &failures {
        println!("FAILED {f}");
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
