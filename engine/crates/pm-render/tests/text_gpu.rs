//! The GPU text path against its CPU reference and itself:
//!
//! - the float paint pass (coverage curve, color glyph images, Gaussian
//!   blur, item order) equals `pm_text::render::paint` within float
//!   rounding when both read the same `vello_cpu` coverage;
//! - shared R32Float area coverage and the CPU coverage fallback agree (SSIM);
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
        Box::new(support::text::MainThreadFonts(
            pm_eval::text_measure::CompatMeasure::new(pm_text::registry::CoreTextRegistry::new()),
            None,
        ))
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
        let a = gpu
            .text_raster(&doc, "Ptest-root", "l", *t, *scale, Precision::Float32)
            .unwrap()
            .unwrap();
        let b = cpu
            .text_raster(&doc, "Ptest-root", "l", *t, *scale, Precision::Float32)
            .unwrap()
            .unwrap();
        assert_eq!(
            a.plan.key, b.plan.key,
            "{name}: the plan does not depend on the coverage source"
        );
        // Paint pass vs the CPU reference over the same (vello_cpu) coverage.
        let reference = pm_render::text::gpu::render_cpu(&b.plan);
        let max = b
            .pixels
            .iter()
            .zip(&reference.data)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        println!(
            "{name:<8} {}x{} paint pass vs CPU reference: max |Δ| {max:.6}",
            b.plan.width(),
            b.plan.height()
        );
        if max > 1.0 / 255.0 {
            failures.push(format!(
                "{name}: paint pass differs from the CPU reference by {max}"
            ));
        }
        // GPU coverage vs the CPU fallback.
        let (w, h) = (a.plan.width() as usize, a.plan.height() as usize);
        let rgba8 = |p: &[f32]| -> Vec<u8> {
            p.iter()
                .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
                .collect()
        };
        let (ink, _, ratio) =
            pm_render::parity::text_ssim(&rgba8(&b.pixels), &rgba8(&a.pixels), w, h);
        println!(
            "{name:<8} R32Float area vs CPU coverage: ink SSIM {ink:.5}, ink ratio {ratio:.4}"
        );
        if ink < 0.995 {
            failures.push(format!("{name}: GPU and CPU coverage differ (SSIM {ink})"));
        }
        // Determinism and the raster cache.
        let hits = gpu.text_gpu().stats.cache_hits;
        let again = gpu
            .text_raster(&doc, "Ptest-root", "l", *t, *scale, Precision::Float32)
            .unwrap()
            .unwrap();
        assert_eq!(again.pixels, a.pixels, "{name}: deterministic");
        assert_eq!(
            gpu.text_gpu().stats.cache_hits,
            hits + 1,
            "{name}: served from the cache"
        );
        gpu.text_gpu().clear();
        let fresh = gpu
            .text_raster(&doc, "Ptest-root", "l", *t, *scale, Precision::Float32)
            .unwrap()
            .unwrap();
        assert_eq!(fresh.pixels, a.pixels, "{name}: repainted identically");
    }
    // Glyph images beyond the atlas: an 80 degree skew of 75 px text at 2x
    // (150 device px) and a 45 degree rotation of a Zapfino glyph whose
    // upright outline fits a plot. Skia draws both as paths; neither may
    // reach the A8 atlas (which panicked on masks over 512 px).
    let animated = |text: &str, font: &str, size: f64, p: serde_json::Value| {
        json!({
            "text": text, "font": font, "weight": 400, "size": size, "color": "#101010",
            "align": "left", "italic": false,
            "animators": [{"id": "a1", "name": "transform", "enabled": true, "mode": "stagger",
                "unit": "characters", "order": "forward", "direction": "in", "easing": "linear",
                "p": p}]
        })
    };
    let stagger = |key: &str, value: f64| {
        let mut p = json!({"amount": ch(json!(100)), "delay": ch(json!(0)),
            "duration": ch(json!(1)), "stagger": ch(json!(0))});
        p[key] = ch(json!(value));
        p
    };
    let oversized = [
        (
            "skew80",
            animated("Hi", "Arial", 75.0, stagger("skew", 80.0)),
        ),
        (
            "rotated",
            animated("g", "Zapfino", 50.0, stagger("rotation", 45.0)),
        ),
    ];
    for (name, d) in &oversized {
        let doc = doc_of("text", d);
        let raster = |r: &mut Renderer| {
            r.text_raster(&doc, "Ptest-root", "l", 0.0, 2.0, Precision::Float32)
                .unwrap()
                .unwrap()
        };
        let a = raster(&mut gpu);
        let ink = a.pixels.chunks_exact(4).filter(|p| p[3] > 0.5).count();
        println!(
            "{name:<8} {}x{} beyond-atlas glyphs: {ink} inked px",
            a.plan.width(),
            a.plan.height()
        );
        if ink == 0 {
            failures.push(format!("{name}: the oversized glyph drew nothing"));
        }
        if a.plan.scene.items.iter().any(|i| {
            !matches!(&i.content, pm_text::render::scene::ItemContent::Coverage { paths }
                if !paths.is_empty())
        }) {
            failures.push(format!(
                "{name}: transformed glyph left the outline coverage path"
            ));
        }
        gpu.text_gpu().clear();
        assert_eq!(raster(&mut gpu).pixels, a.pixels, "{name}: deterministic");
    }
    // Without fonts a text layer is refused.
    let mut bare = Renderer::headless().expect("a Metal device");
    let doc = doc_of("text", &docs[1].2);
    let request = pm_render::RenderRequest::sized(
        pm_protocol::common::Size::new(320, 180),
        Precision::Float16,
    );
    match bare.render(
        &doc,
        "Ptest-root",
        support::ticks(0.0),
        &request,
        &pm_render::MapResolver::default(),
    ) {
        Err(RenderError::Unsupported(u)) => {
            assert!(
                u.iter()
                    .any(|u| matches!(u.feature, pm_render::Feature::Text(_))),
                "{u:?}"
            );
        }
        Ok(_) => failures.push("a text layer rendered without fonts".into()),
        Err(e) => failures.push(format!("unexpected error without fonts: {e}")),
    }
    linear_overlap_and_colr_source(&mut gpu, &mut failures);
    linear_project_text_source(&mut gpu, &mut failures);
    for f in &failures {
        println!("FAILED {f}");
    }
    assert!(failures.is_empty());
}

/// Two translucent glyphs of different colors overlap in the project
/// working space. The synthetic COLR source has palette alpha in an already
/// decoded working image; the shader must not decode it a second time.
#[cfg(target_os = "macos")]
fn linear_overlap_and_colr_source(renderer: &mut pm_render::Renderer, failures: &mut Vec<String>) {
    use kurbo::{Affine, Rect};
    use pm_protocol::color::{Precision, WorkingEncoding};
    use pm_text::render::colr::RgbaF32;
    use pm_text::render::scene::{ImagePixels, Item, ItemContent, TextScene};
    use std::sync::Arc;

    let layer = support::doc::layer(
        "t",
        "text",
        serde_json::json!({
            "text": "H", "font": "SF Pro Display", "size": 48,
            "weight": 600, "color": "#ffffff"
        }),
        64.0,
        48.0,
        3.0,
    );
    let document = support::doc::project(128, 96, 3.0, support::doc::no_fill(), vec![layer]);
    let document = pm_model::Document::new(
        Some(pm_model::json::parse(&document.to_string()).unwrap()),
        1,
    );
    let raster = renderer
        .text_raster(&document, "Ptest-root", "t", 0.0, 1.0, Precision::Float32)
        .unwrap()
        .unwrap();
    let mut glyph = raster
        .plan
        .scene
        .items
        .iter()
        .find(|item| matches!(&item.content, ItemContent::Coverage { paths } if !paths.is_empty()))
        .expect("outlined H")
        .clone();
    let move_to = Affine::translate((38.25 - glyph.bounds.x0, 21.5 - glyph.bounds.y0));
    glyph.bounds = move_to.transform_rect_bbox(glyph.bounds);
    if let ItemContent::Coverage { paths } = &mut glyph.content {
        for path in paths {
            path.transform = move_to * path.transform;
        }
    }
    glyph.color = [0.95, 0.08, 0.15, 0.55];
    let mut blue = glyph.clone();
    blue.color = [0.07, 0.28, 0.95, 0.45];
    let colr = Item {
        content: ItemContent::Image {
            image: ImagePixels::WorkingF32(Arc::new(RgbaF32 {
                width: 8,
                height: 8,
                // A constant palette color with 40% alpha, premultiplied
                // in the already-linear project working domain.
                data: [0.3f32, 0.08, 0.02, 0.4].repeat(64),
            })),
            transform: Affine::translate((112.0, 80.0)),
        },
        color: [1.0; 4],
        opacity: 1.0,
        blur: 0.0,
        bounds: Rect::new(112.0, 80.0, 120.0, 88.0),
    };
    let scene = TextScene {
        width: 128,
        height: 96,
        items: vec![glyph, blue, colr],
    };
    let ideal = support::text_reference::render_in_working(&scene, WorkingEncoding::LinearLight);
    let colr_pixel = 83 * 128 + 115;
    if ideal.unreferenced == 0
        || ideal.classes[colr_pixel] != pm_reference::compare::Class::Unreferenced
        || !ideal.rgba[colr_pixel * 4].is_nan()
    {
        failures.push("COLR graph pixels were incorrectly used as independent reference".into());
    }
    let plan = pm_render::text::TextPlan::whole(scene);
    let device = renderer.device();
    let queue = renderer.queue();
    let mut painter = pm_render::text::gpu::TextGpu::new(device);
    let mut pool = pm_gpu::TargetPool::new();
    let mut held = Vec::new();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("linear overlapping text gate"),
    });
    let view = painter
        .render(
            device,
            queue,
            &mut encoder,
            &mut pool,
            &mut held,
            &plan,
            Precision::Float32,
            WorkingEncoding::LinearLight,
        )
        .unwrap();
    queue.submit([encoder.finish()]);
    for target in held {
        pool.give(target);
    }
    let actual = pm_gpu::readback::read_f32(device, queue, view.texture()).unwrap();
    let (mut overlap, mut max_cv) = (0usize, 0.0f64);
    for (i, (a, r)) in actual
        .chunks_exact(4)
        .zip(ideal.rgba.chunks_exact(4))
        .enumerate()
    {
        if ideal.classes[i] == pm_reference::compare::Class::Unreferenced {
            continue;
        }
        // Fully covered portions of both glyphs isolate paint-domain math
        // from the independently quantified area coverage at glyph edges.
        if r[3] < 0.7524 {
            continue;
        }
        overlap += 1;
        for c in 0..4 {
            max_cv = max_cv.max((f64::from(a[c]) - r[c]).abs() * 255.0);
        }
    }
    println!("case 0: linear glyph overlap {overlap} pixels, max {max_cv:.3} CV");
    if overlap < 20 || max_cv > 1.0 {
        failures.push(format!(
            "linear glyph overlap {overlap} pixels, max {max_cv:.3} CV"
        ));
    }
    let offset = colr_pixel * 4;
    let expected = [0.3f64, 0.08, 0.02, 0.4];
    let image_max = (0..4)
        .map(|c| (f64::from(actual[offset + c]) - expected[c]).abs() * 255.0)
        .fold(0.0f64, f64::max);
    println!("case 1: working COLR palette alpha max {image_max:.3} CV");
    if image_max > 1.0 {
        failures.push(format!("working COLR palette alpha max {image_max:.3} CV"));
    }
}

/// Exercise the actual project `Source::Text` execution path, including its
/// encoded=false texture flag. Two translucent midtone glyphs overlap in the
/// project's linear working domain; a second display decode changes them by
/// far more than one code value.
#[cfg(target_os = "macos")]
fn linear_project_text_source(renderer: &mut pm_render::Renderer, failures: &mut Vec<String>) {
    use pm_protocol::color::{Precision, WorkingEncoding};
    use pm_protocol::common::Size;
    use pm_reference::composite::{Blend, blend, scale};
    use pm_render::plan::{Content, Source};
    use serde_json::json;

    let glyph = |id: &str, color: &str| {
        support::doc::layer(
            id,
            "text",
            json!({"text":"H", "font":"SF Pro Display", "size":48,
                "weight":600, "color":color, "align":"left"}),
            38.0,
            28.0,
            3.0,
        )
    };
    let mut project = support::doc::project(
        128,
        96,
        3.0,
        support::doc::no_fill(),
        vec![glyph("top", "#4080C099"), glyph("bottom", "#A0604099")],
    );
    project["color"] = json!({"mode":"standard", "linearBlending":true,
        "precision":"float32", "blendDomain":"hdrSafe"});
    project["colorDepth"] = json!(32);
    let doc = pm_model::Document::new(
        Some(pm_model::json::parse(&project.to_string()).unwrap()),
        1,
    );
    let working = pm_render::composite::color::project_color(doc.root().unwrap())
        .unwrap()
        .working();
    assert_eq!(working, WorkingEncoding::LinearLight);
    let request = pm_render::RenderRequest::sized(Size::new(128, 96), Precision::Float32);
    let resolver = pm_render::MapResolver::default();
    let (plan, plan_failures) = renderer
        .plan(&doc, "Ptest-root", support::ticks(0.0), &request, &resolver)
        .unwrap();
    assert!(plan_failures.is_empty(), "{plan_failures:?}");
    let mut ideal = vec![0.0f64; 128 * 96 * 4];
    let mut layers = 0usize;
    let mut overlap = vec![0u8; 128 * 96];
    for draw in &plan.draws {
        let Content::Texture {
            source: Source::Text(text),
            alpha,
            ..
        } = &draw.content
        else {
            continue;
        };
        layers += 1;
        assert_eq!((text.scene.width, text.scene.height), (128, 96));
        let layer = support::text_reference::render_in_working(&text.scene, working);
        assert_eq!(layer.unreferenced, 0);
        for (i, src) in layer.rgba.chunks_exact(4).enumerate() {
            if src[3] > 0.5 {
                overlap[i] += 1;
            }
            let dst = &mut ideal[i * 4..][..4];
            let result = blend(
                Blend::Normal,
                scale([src[0], src[1], src[2], src[3]], *alpha),
                [dst[0], dst[1], dst[2], dst[3]],
            );
            dst.copy_from_slice(&result);
        }
    }
    assert_eq!(layers, 2, "both text layers use Source::Text");
    let frame = renderer
        .render(&doc, "Ptest-root", support::ticks(0.0), &request, &resolver)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let actual = renderer.read_rgba(&frame).unwrap();
    // The renderer's final EncodeFlip writes premultiplied E even when its
    // intermediate composition used linear light. Encode the completed f64
    // reference once, after all source-over operations.
    let output: Vec<f64> = ideal
        .chunks_exact(4)
        .flat_map(|p| {
            let alpha = p[3];
            [
                if alpha > 0.0 {
                    pm_reference::color::Transfer::Srgb.from_linear(p[0] / alpha) * alpha
                } else {
                    0.0
                },
                if alpha > 0.0 {
                    pm_reference::color::Transfer::Srgb.from_linear(p[1] / alpha) * alpha
                } else {
                    0.0
                },
                if alpha > 0.0 {
                    pm_reference::color::Transfer::Srgb.from_linear(p[2] / alpha) * alpha
                } else {
                    0.0
                },
                alpha,
            ]
        })
        .collect();
    let mut max_cv = 0.0f64;
    let mut worst = (0usize, [0.0f64; 4], [0.0f64; 4]);
    let mut overlap_pixels = 0usize;
    let mut midtones = 0usize;
    for (i, (a, r)) in actual
        .chunks_exact(4)
        .zip(output.chunks_exact(4))
        .enumerate()
    {
        overlap_pixels += usize::from(overlap[i] == 2);
        midtones += usize::from(ideal[i * 4 + 3] > 0.5 && (0.05..0.5).contains(&ideal[i * 4]));
        for c in 0..4 {
            let delta = (f64::from(a[c]) - r[c]).abs() * 255.0;
            if delta > max_cv {
                max_cv = delta;
                worst = (
                    i,
                    std::array::from_fn(|channel| f64::from(a[channel])),
                    r.try_into().unwrap(),
                );
            }
        }
    }
    println!(
        "case 2: linear project Source::Text {overlap_pixels} overlap pixels, {midtones} midtones, max {max_cv:.3} CV"
    );
    if max_cv > 1.0 {
        println!(
            "case 2 worst pixel ({},{}): actual {:?}, independent encoded {:?}, independent working {:?}",
            worst.0 % 128,
            worst.0 / 128,
            worst.1,
            worst.2,
            &ideal[worst.0 * 4..worst.0 * 4 + 4],
        );
    }
    if overlap_pixels < 20 || midtones < 20 || max_cv > 1.0 {
        failures.push(format!(
            "linear project Source::Text overlap {overlap_pixels}, midtones {midtones}, max {max_cv:.3} CV"
        ));
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {}
