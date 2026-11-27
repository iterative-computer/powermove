//! Focused contracts for the native SVG float painter.

use pm_protocol::color::Precision;
use pm_render::vector::svg_float::SvgPainter;
use pm_vector::kurbo::Affine;
use pm_vector::render::scene::PixelRect;
use pm_vector::render::svg::SvgDocument;

fn render(svg: &str, w: u32, h: u32) -> Vec<f32> {
    let gpu = pm_gpu::Gpu::new("SVG float test").unwrap();
    let doc = SvgDocument::parse(svg.as_bytes()).unwrap();
    let mut painter = SvgPainter::new(&gpu.device).unwrap();
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
fn missing_float_blending_selects_fallback_before_pipeline_creation() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::METAL;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    // Deliberately omit the optional float features even when the adapter
    // has them. Pipeline creation must never trigger a validation panic.
    let (device, _) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    assert!(SvgPainter::new(&device).is_none());
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
    // The midpoint is not quantized to an 8-bit ramp entry.
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
fn nested_clips_share_subpixels_with_their_children() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><defs><clipPath id="x"><rect x="0.5" width="1.5" height="2"/></clipPath><clipPath id="y"><rect y="0.5" width="2" height="1.5"/></clipPath></defs><g clip-path="url(#x)"><g clip-path="url(#y)"><rect width="2" height="2" fill="#fff"/><rect x="0.5" width="1.5" height="2" fill="#000"/></g></g></svg>"##;
    let actual = pixel(&render(svg, 2, 2), 2, 0, 0);
    // At (0,0), x>=0.5 and y>=0.5 occupy one quarter of the pixel. The
    // dark child covers precisely that intersection, so no white remains.
    for channel in &actual[..3] {
        assert!(channel.abs() < 1e-5, "{actual:?}");
    }
    assert!((actual[3] - 0.25).abs() < 1e-5, "{actual:?}");
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
fn coincident_gradient_stops_integrate_the_color_step() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="8"><defs><linearGradient id="g"><stop offset="0.515625" stop-color="#f00"/><stop offset="0.515625" stop-color="#00f"/></linearGradient></defs><rect width="32" height="8" fill="url(#g)"/></svg>"##;
    let pixels = render(svg, 32, 8);
    // The step crosses pixel 16 at its center: half red, half blue. The
    // right-hand stop owns the exact offset, which has zero area itself.
    assert_eq!(pixel(&pixels, 32, 16, 4), [0.5, 0.0, 0.5, 1.0]);
    assert_eq!(pixel(&pixels, 32, 15, 4), [1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn radial_paint_is_integrated_with_circle_coverage() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="54" viewBox="0 0 320 180"><defs><linearGradient id="a" x1="0" y1="0" x2="1" y2="1"><stop stop-color="#1B2A4A"/><stop offset="0.6" stop-color="#4C8DFF"/><stop offset="1" stop-color="#FFD66B"/></linearGradient><radialGradient id="b" cx="0.5" cy="0.5" r="0.5"><stop stop-color="#FFF4D0"/><stop offset="1" stop-color="#FF6B1A" stop-opacity="0"/></radialGradient></defs><rect width="320" height="180" fill="url(#a)"/><circle cx="230" cy="70" r="46" fill="url(#b)"/></svg>"##;
    let actual = pixel(&render(svg, 96, 54), 96, 82, 19);
    // Independent f64 midpoint integration in SVG user coordinates. This
    // pixel straddles the circle and a transparent radial stop; sampling its
    // paint only at the center gives a visibly different blue value.
    let (dark, blue, gold) = (
        [27.0, 42.0, 74.0],
        [76.0, 141.0, 255.0],
        [255.0, 214.0, 107.0],
    );
    let (center, outer) = ([255.0, 244.0, 208.0], [255.0, 107.0, 26.0]);
    let mut expected = [0.0f64; 3];
    const N: usize = 512;
    for sy in 0..N {
        for sx in 0..N {
            let x = (82.0 + (sx as f64 + 0.5) / N as f64) / 0.3;
            let y = (19.0 + (sy as f64 + 0.5) / N as f64) / 0.3;
            let linear = ((x / 320.0 + y / 180.0) * 0.5).clamp(0.0, 1.0);
            let radial = ((x - 230.0).hypot(y - 70.0) / 46.0).clamp(0.0, 1.0);
            let (a, b, t) = if linear < 0.6 {
                (dark, blue, linear / 0.6)
            } else {
                (blue, gold, (linear - 0.6) / 0.4)
            };
            for k in 0..3 {
                let background = a[k] + (b[k] - a[k]) * t;
                let foreground = center[k] + (outer[k] - center[k]) * radial;
                expected[k] += (foreground * (1.0 - radial) + background * radial) / (N * N) as f64;
            }
        }
    }
    for k in 0..3 {
        assert!(
            (f64::from(actual[k]) * 255.0 - expected[k]).abs() < 0.15,
            "channel {k}: {actual:?} vs {expected:?}"
        );
    }
}

#[test]
fn near_tangent_focal_gradient_integrates_instead_of_sampling_its_center() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><defs><radialGradient id="g" cx="32" cy="32" fx="47.5" fy="32" r="16" gradientUnits="userSpaceOnUse"><stop stop-color="#000"/><stop offset="1" stop-color="#fff"/></radialGradient></defs><rect width="64" height="64" fill="url(#g)"/></svg>"##;
    let actual = pixel(&render(svg, 64, 64), 64, 47, 32)[0] as f64;
    // Solve the authored two-circle radial equation in f64 at many points
    // across the pixel. Its focal point is only 0.5 px inside the outer
    // circle, so the radial derivative is far larger than 1/r here.
    const N: usize = 512;
    let mut expected = 0.0;
    for iy in 0..N {
        for ix in 0..N {
            let x = 47.0 + (ix as f64 + 0.5) / N as f64;
            let y = 32.0 + (iy as f64 + 0.5) / N as f64;
            let ex = x - 47.5;
            let ey = y - 32.0;
            // |(ex,ey) + (15.5,0)t|² = (16t)².
            let a = 15.5f64.powi(2) - 16.0f64.powi(2);
            let b = 31.0 * ex;
            let c = ex * ex + ey * ey;
            let discriminant = (b * b - 4.0 * a * c).sqrt();
            let t = ((-b - discriminant) / (2.0 * a))
                .max((-b + discriminant) / (2.0 * a))
                .clamp(0.0, 1.0);
            expected += t / (N * N) as f64;
        }
    }
    assert!((actual - expected).abs() < 0.01, "{actual} vs {expected}");
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
