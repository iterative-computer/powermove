//! The GPU text path against its CPU reference and itself:
//!
//! - the float paint pass (coverage curve, color glyph images, Gaussian
//!   blur, item order) equals `pm_text::render::paint` within float
//!   rounding when both read the same `vello_cpu` coverage;
//! - Vello's GPU coverage and the `vello_cpu` fallback agree (SSIM);
//! - rasters are deterministic and the content-keyed cache returns them;
//! - without fonts a text layer is refused, not drawn wrong.
//!
//! Runs without the harness (the CoreText registry needs the main thread).

mod support;

#[cfg(target_os = "macos")]
fn main() {
    use pm_protocol::color::Precision;
    use pm_render::text::gpu::Coverage;
    use pm_render::{RenderError, Renderer};
    use serde_json::json;
    use support::doc::{ch, layer, no_fill, project};

    let fonts = || {
        Box::new(support::text::MainThreadFonts(pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new()), None))
    };
    let blur = json!({
        "text": "Blur in\nsecond line", "boxWidth": ch(json!(0)), "boxHeight": ch(json!(0)),
        "font": "SF Pro Display", "weight": 700, "size": 44, "tracking": -2, "leading": 1.2,
        "color": "#F2F2F2", "align": "center", "italic": false,
        "animators": [{"id": "a1", "name": "blur", "enabled": true, "mode": "stagger", "unit": "characters",
            "order": "forward", "direction": "in", "easing": "cubicOut", "p": {
                "amount": ch(json!(100)), "delay": ch(json!(0)), "duration": ch(json!(0.8)),
                "stagger": ch(json!(0.03)), "opacity": ch(json!(0)), "blur": ch(json!(11))}}]
    });
    let emoji = json!({
        "text": "Emoji 👋🏽 🇰🇷 ❤️ and text", "font": "SF Pro Display", "weight": 500, "size": 36,
        "color": "#FF3B30", "align": "left"
    });
    let caption = json!({
        "cues": [{"id": "c1", "text": "Stroked caption with a highlight", "start": 0, "end": 3}],
        "style": {"preset": "pop", "font": "SF Pro Display", "weight": 900, "italic": false, "size": 64,
            "tracking": 0, "leading": 1.05, "fill": "#FFFFFF", "stroke": 7, "strokeColor": "#000000",
            "box": true, "boxColor": "#202020", "boxOpacity": 60, "boxPadding": 14, "boxRadius": 10,
            "placement": "middle", "align": "center", "offsetX": 0, "offsetY": 0, "safeMargin": 8,
            "maxWidth": 76, "maxLines": 2, "textCase": "upper", "highlight": true,
            "highlightColor": "#FFD43B", "highlightScale": 116, "wordAnimation": "pop"}
    });
    let docs = [
        ("blur", "text", blur, 0.3, 1.5),
        ("emoji", "text", emoji, 0.0, 1.0),
        ("caption", "captions", caption, 1.05, 0.75),
    ];
    let doc_of = |ty: &str, d: &serde_json::Value| {
        let l = layer("l", ty, d.clone(), 960.0, 540.0, 10.0);
        let v = project(1920, 1080, 10.0, no_fill(), vec![l]);
        pm_model::Document::new(Some(pm_model::json::parse(&v.to_string()).unwrap()), 1)
    };
    let mut gpu = Renderer::headless().expect("a Metal device");
    gpu.set_text_fonts(Some(fonts()));
    let mut cpu = Renderer::headless().expect("a Metal device");
    cpu.set_text_fonts(Some(fonts()));
    cpu.text_gpu().set_coverage(Coverage::Cpu);
    let mut failures = Vec::new();
    for (name, ty, d, t, scale) in &docs {
        let doc = doc_of(ty, d);
        let a = gpu.text_raster(&doc, "Ptest-root", "l", *t, *scale, Precision::Float32).unwrap().unwrap();
        let b = cpu.text_raster(&doc, "Ptest-root", "l", *t, *scale, Precision::Float32).unwrap().unwrap();
        assert_eq!(a.plan.key, b.plan.key, "{name}: the plan does not depend on the coverage source");
        // Paint pass vs the CPU reference over the same (vello_cpu) coverage.
        let reference = pm_render::text::gpu::render_cpu(&b.plan);
        let max = b
            .pixels
            .iter()
            .zip(&reference.data)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        println!("{name:<8} {}x{} paint pass vs CPU reference: max |Δ| {max:.6}", b.plan.width(), b.plan.height());
        if max > 1.0 / 255.0 {
            failures.push(format!("{name}: paint pass differs from the CPU reference by {max}"));
        }
        // GPU coverage vs the CPU fallback.
        let (w, h) = (a.plan.width() as usize, a.plan.height() as usize);
        let rgba8 = |p: &[f32]| -> Vec<u8> { p.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8).collect() };
        let (ink, _, ratio) = pm_render::parity::text_ssim(&rgba8(&b.pixels), &rgba8(&a.pixels), w, h);
        println!("{name:<8} Vello GPU vs vello_cpu coverage: ink SSIM {ink:.5}, ink ratio {ratio:.4}");
        if ink < 0.995 {
            failures.push(format!("{name}: GPU and CPU coverage differ (SSIM {ink})"));
        }
        // Determinism and the raster cache.
        let hits = gpu.text_gpu().stats.cache_hits;
        let again = gpu.text_raster(&doc, "Ptest-root", "l", *t, *scale, Precision::Float32).unwrap().unwrap();
        assert_eq!(again.pixels, a.pixels, "{name}: deterministic");
        assert_eq!(gpu.text_gpu().stats.cache_hits, hits + 1, "{name}: served from the cache");
        gpu.text_gpu().clear();
        let fresh = gpu.text_raster(&doc, "Ptest-root", "l", *t, *scale, Precision::Float32).unwrap().unwrap();
        assert_eq!(fresh.pixels, a.pixels, "{name}: repainted identically");
    }
    // Without fonts a text layer is refused.
    let mut bare = Renderer::headless().expect("a Metal device");
    let doc = doc_of("text", &docs[1].2);
    let request = pm_render::RenderRequest::sized(pm_protocol::common::Size::new(320, 180), Precision::Float16);
    match bare.render(&doc, "Ptest-root", support::ticks(0.0), &request, &pm_render::MapResolver::default()) {
        Err(RenderError::Unsupported(u)) => {
            assert!(u.iter().any(|u| matches!(u.feature, pm_render::Feature::Text(_))), "{u:?}");
        }
        Ok(_) => failures.push("a text layer rendered without fonts".into()),
        Err(e) => failures.push(format!("unexpected error without fonts: {e}")),
    }
    for f in &failures {
        println!("FAILED {f}");
    }
    assert!(failures.is_empty());
}

#[cfg(not(target_os = "macos"))]
fn main() {}
