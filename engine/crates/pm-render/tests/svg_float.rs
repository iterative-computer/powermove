//! Focused contracts for the native SVG float painter.

use pm_protocol::color::Precision;
use pm_render::vector::svg_float::SvgPainter;
use pm_vector::kurbo::Affine;
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;

fn render(svg: &str, w: u32, h: u32) -> Vec<f32> {
    let gpu = pm_gpu::Gpu::new("SVG float test").unwrap();
    let doc = SvgDocument::parse(svg.as_bytes()).unwrap();
    let mut painter = SvgPainter::new(&gpu.device);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let (texture, _) = painter
        .render(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &doc,
            Affine::IDENTITY,
            PixelRect::new(0, 0, w, h),
            &[],
            Precision::Float32,
        )
        .unwrap()
        .unwrap();
    gpu.queue.submit([encoder.finish()]);
    pm_gpu::readback::read_f32(&gpu.device, &gpu.queue, &texture).unwrap()
}

fn pixel(pixels: &[f32], w: usize, x: usize, y: usize) -> [f32; 4] {
    pixels[(y * w + x) * 4..][..4].try_into().unwrap()
}

#[test]
fn gradients_interpolate_straight_alpha_in_float() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="256" height="8"><defs><linearGradient id="g" x1="0" y1="0" x2="256" y2="0" gradientUnits="userSpaceOnUse"><stop stop-color="#ff0000" stop-opacity="0"/><stop offset="1" stop-color="#0000ff"/></linearGradient></defs><rect width="256" height="8" fill="url(#g)"/></svg>"##;
    let pixels = render(svg, 256, 8);
    let c = pixel(&pixels, 256, 128, 4);
    let t = 128.5 / 256.0;
    assert!((c[0] - (1.0 - t) * t).abs() < 0.002, "{c:?}");
    assert!((c[2] - t * t).abs() < 0.002, "{c:?}");
    assert!((c[3] - t).abs() < 0.002, "{c:?}");
    // The midpoint is not quantized to a Vello 8-bit ramp entry.
    let quarter = pixel(&pixels, 256, 64, 4);
    assert!((quarter[3] * 255.0 - (quarter[3] * 255.0).round()).abs() > 0.05);
}

#[test]
fn isolated_opacity_and_clip_apply_after_child_compositing() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8"><defs><clipPath id="c"><rect x="0" y="0" width="8" height="8"/></clipPath></defs><g opacity="0.5" clip-path="url(#c)"><rect width="16" height="8" fill="#ff0000"/><rect width="16" height="8" fill="#0000ff"/></g></svg>"##;
    let pixels = render(svg, 16, 8);
    let inside = pixel(&pixels, 16, 4, 4);
    let outside = pixel(&pixels, 16, 12, 4);
    assert!((inside[2] - 0.5).abs() < 0.002, "{inside:?}");
    assert!(inside[0].abs() < 0.002, "{inside:?}");
    assert!((inside[3] - 0.5).abs() < 0.002, "{inside:?}");
    assert!(outside.iter().all(|v| v.abs() < 0.002), "{outside:?}");
}

#[test]
fn luminance_mask_uses_painted_color() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><defs><mask id="m"><rect width="8" height="8" fill="#808080"/></mask></defs><g mask="url(#m)"><rect width="8" height="8" fill="#00ff00"/></g></svg>"##;
    let c = pixel(&render(svg, 8, 8), 8, 4, 4);
    assert!((c[1] - 128.0 / 255.0).abs() < 0.01, "{c:?}");
    assert!((c[3] - 128.0 / 255.0).abs() < 0.01, "{c:?}");
}

#[test]
fn radial_focus_and_repeat_spread_use_gradient_space() {
    let radial = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="12"><defs><radialGradient id="g" cx="8" cy="6" fx="4" fy="6" r="6" gradientUnits="userSpaceOnUse"><stop stop-color="#000000"/><stop offset="1" stop-color="#ffffff"/></radialGradient></defs><rect width="16" height="12" fill="url(#g)"/></svg>"##;
    let p = render(radial, 16, 12);
    // At y=6.5, solve |P - (F+t(C-F))| = 6t independently.
    let x = 7.5f64;
    let y = 6.5f64;
    let a = 4.0f64 * 4.0 - 6.0 * 6.0;
    let b = -2.0 * ((x - 4.0) * 4.0);
    let c = (x - 4.0).powi(2) + (y - 6.0).powi(2);
    let t = ((-b - (b * b - 4.0 * a * c).sqrt()) / (2.0 * a))
        .max((-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a));
    let q = pixel(&p, 16, 7, 6);
    assert!((f64::from(q[0]) - t).abs() < 0.002, "{q:?}, t={t}");

    let repeat = r##"<svg xmlns="http://www.w3.org/2000/svg" width="12" height="4"><defs><linearGradient id="g" x1="0" y1="0" x2="4" y2="0" gradientUnits="userSpaceOnUse" spreadMethod="repeat"><stop stop-color="#000000"/><stop offset="1" stop-color="#ffffff"/></linearGradient></defs><rect width="12" height="4" fill="url(#g)"/></svg>"##;
    let q = pixel(&render(repeat, 12, 4), 12, 6, 2);
    assert!((q[0] - 0.625).abs() < 0.002, "{q:?}");
}

#[test]
fn blend_mode_composites_in_premultiplied_float() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#ff0000"/><g style="mix-blend-mode:multiply" opacity="0.5"><rect width="8" height="8" fill="#0000ff"/></g></svg>"##;
    let q = pixel(&render(svg, 8, 8), 8, 4, 4);
    assert!((q[0] - 0.5).abs() < 0.002, "{q:?}");
    assert!(q[2].abs() < 0.002, "{q:?}");
    assert!((q[3] - 1.0).abs() < 0.002, "{q:?}");
}

#[test]
fn alpha_masks_keep_gradient_stop_opacity() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="8"><defs><linearGradient id="g"><stop stop-color="#f00" stop-opacity="0"/><stop offset="1" stop-color="#0f0"/></linearGradient><mask id="m" mask-type="alpha"><rect width="32" height="8" fill="url(#g)"/></mask></defs><rect width="32" height="8" fill="#fff" mask="url(#m)"/></svg>"##;
    let pixels = render(svg, 32, 8);
    for x in [4, 16, 28] {
        let p = pixel(&pixels, 32, x, 4);
        let alpha = (x as f32 + 0.5) / 32.0;
        assert!((p[3] - alpha).abs() < 1e-5, "{x}: {p:?}");
    }
}

#[test]
fn coincident_gradient_stops_use_the_last_color_at_the_offset() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="8"><defs><linearGradient id="g"><stop offset="0.515625" stop-color="#f00"/><stop offset="0.515625" stop-color="#00f"/></linearGradient></defs><rect width="32" height="8" fill="url(#g)"/></svg>"##;
    let pixels = render(svg, 32, 8);
    assert_eq!(pixel(&pixels, 32, 16, 4), [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(pixel(&pixels, 32, 15, 4), [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn separable_blends_follow_the_f64_formula_including_endpoint_cases() {
    use pm_reference::composite::{Blend, blend};
    let modes = [
        ("normal", Blend::Normal),
        ("multiply", Blend::Multiply),
        ("screen", Blend::Screen),
        ("overlay", Blend::Overlay),
        ("darken", Blend::Darken),
        ("lighten", Blend::Lighten),
        ("color-dodge", Blend::ColorDodge),
        ("color-burn", Blend::ColorBurn),
        ("hard-light", Blend::HardLight),
        ("soft-light", Blend::SoftLight),
        ("difference", Blend::Difference),
        ("exclusion", Blend::Exclusion),
    ];
    for (name, mode) in modes {
        for (back, front) in [
            ([0u8, 64, 255], [255u8, 0, 128]),
            ([255, 0, 0], [0, 255, 255]),
        ] {
            let css = |c: [u8; 3]| format!("rgb({},{},{})", c[0], c[1], c[2]);
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="{}"/><g style="mix-blend-mode:{name}" opacity="0.6"><rect width="8" height="8" fill="{}"/></g></svg>"##,
                css(back),
                css(front)
            );
            let actual = pixel(&render(&svg, 8, 8), 8, 4, 4);
            let expected = blend(
                mode,
                [
                    f64::from(front[0]) / 255.0 * 0.6,
                    f64::from(front[1]) / 255.0 * 0.6,
                    f64::from(front[2]) / 255.0 * 0.6,
                    0.6,
                ],
                [
                    f64::from(back[0]) / 255.0,
                    f64::from(back[1]) / 255.0,
                    f64::from(back[2]) / 255.0,
                    1.0,
                ],
            );
            for k in 0..4 {
                assert!(
                    (f64::from(actual[k]) - expected[k]).abs() < 2e-6,
                    "{name} {back:?} {front:?}: {actual:?} vs {expected:?}"
                );
            }
        }
    }
}
