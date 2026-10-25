//! Project depth controls float precision. Eight-bit delivery retains extended
//! color in float until its final CSS gamut mapping and byte quantization.
mod support;

use pm_gpu::Target;
use pm_protocol::{color::Precision, common::Size};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;

fn document(depth: u32) -> serde_json::Value {
    let mut l = layer(
        "a",
        "solid",
        json!({"color":"#C58AEAB3","w":38,"h":21}),
        9.0,
        5.0,
        1.0,
    );
    set(&mut l, "opacity", json!(61));
    l["blend"] = json!("screen");
    l["masks"] = json!([{"id":"m","shape":"ellipse","mode":"add","on":true,"p":{
        "x":ch(json!(19)),"y":ch(json!(10)),"w":ch(json!(32)),"h":ch(json!(19)),
        "rotation":ch(json!(11)),"feather":ch(json!(8))}}]);
    let mut d = project(
        64,
        32,
        1.0,
        json!({"type":"linear","angle":21,"stops":[
        {"id":"a","color":"#182132","position":0},{"id":"b","color":"#6D5988","position":100}]}),
        vec![l],
    );
    d["colorDepth"] = json!(depth);
    d
}

#[test]
fn project_depth_overrides_requests_and_rekeys_targets() {
    let mut r = Renderer::headless().unwrap();
    let req = RenderRequest::sized(Size::new(64, 32), Precision::Float32);
    for (depth, format, bpp) in [
        (8, wgpu::TextureFormat::Rgba16Float, 8),
        (32, wgpu::TextureFormat::Rgba32Float, 16),
        (16, wgpu::TextureFormat::Rgba16Float, 8),
        (8, wgpu::TextureFormat::Rgba16Float, 8),
    ] {
        let doc = load(&document(depth));
        assert_eq!(Renderer::target_format(&doc, &req), format);
        let frame = r
            .render(
                &doc,
                "Ptest-root",
                support::ticks(0.0),
                &req,
                &MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        assert_eq!(frame.color.format(), format);
        assert_eq!(frame.color.bytes(), 64 * 32 * bpp);
        let first = r.read_rgba(&frame).unwrap();
        let target = Target::new(
            r.device(),
            "depth caller",
            64,
            32,
            format,
            pm_gpu::target::working_usage(),
        );
        r.render_into(
            &doc,
            "Ptest-root",
            support::ticks(0.0),
            &req,
            &MapResolver::default(),
            &target,
            None,
        )
        .unwrap();
        assert_eq!(
            pm_gpu::readback::read_f32(r.device(), r.queue(), &target.texture).unwrap(),
            first
        );
    }
}

#[test]
fn explicit_depth_replaces_the_old_precision_floor() {
    for depth in [8, 16, 32] {
        let mut d = document(depth);
        d["color"] = json!({"precision":"float32"});
        let doc = load(&d);
        let req = RenderRequest::sized(Size::new(64, 32), Precision::Float32);
        assert_eq!(
            Renderer::target_format(&doc, &req),
            pm_gpu::target::working_format(match depth {
                32 => Precision::Float32,
                _ => Precision::Float16,
            })
        );
    }
}

#[test]
fn eight_bit_delivery_maps_extended_color_after_float_compositing() {
    let shader = layer(
        "extended",
        "shader",
        json!({"code":"void main(){ fragColor = vec4(1.4, 0.0, 0.0, 1.0); }",
            "uniforms":{},"w":8,"h":8}),
        0.0,
        0.0,
        1.0,
    );
    let mut value = project(8, 8, 1.0, no_fill(), vec![shader]);
    value["colorDepth"] = json!(8);
    let doc = load(&value);
    let request = RenderRequest::sized(Size::new(8, 8), Precision::Unorm8);
    let assets = MapResolver::default();
    let mut renderer = Renderer::headless().unwrap();
    let frame = renderer
        .render(&doc, "Ptest-root", support::ticks(0.0), &request, &assets)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let pixels = renderer.read_rgba(&frame).unwrap();
    for p in pixels.chunks_exact(4) {
        assert!(p[0] > 1.39 && p[1] == 0.0 && p[2] == 0.0, "{p:?}");
    }
    let target = Target::new(
        renderer.device(),
        "mapped byte output",
        8,
        8,
        wgpu::TextureFormat::Rgba8Unorm,
        pm_gpu::target::working_usage(),
    );
    let reference = pm_reference::color::css_gamut_map_srgb([1.4, 0.0, 0.0]);
    let expected = reference.map(|v| (v * 255.0).round_ties_even() as u8);
    let error = |rgb: [f64; 3]| {
        rgb.into_iter()
            .zip(reference)
            .map(|(a, b)| (a - b).abs() * 255.0)
            .fold(0.0f64, f64::max)
    };
    eprintln!(
        "CSS output max error: clipped {:.6} CV; mapped {:.6} CV",
        error([1.0, 0.0, 0.0]),
        error(expected.map(|v| f64::from(v) / 255.0))
    );
    for snapshot in [false, true] {
        if snapshot {
            renderer
                .render_snapshot_into(
                    &doc,
                    "Ptest-root",
                    support::ticks(0.0),
                    &request,
                    &assets,
                    &target,
                )
                .unwrap();
        } else {
            renderer
                .render_into(
                    &doc,
                    "Ptest-root",
                    support::ticks(0.0),
                    &request,
                    &assets,
                    &target,
                    None,
                )
                .unwrap();
        }
        let actual =
            pm_gpu::readback::read_f32(renderer.device(), renderer.queue(), &target.texture)
                .unwrap();
        for p in actual.chunks_exact(4) {
            assert_eq!(
                [p[0], p[1], p[2]].map(|v| (v * 255.0).round() as u8),
                expected
            );
        }
    }
}

#[test]
fn scene3d_and_2d_overlay_render_at_every_project_depth() {
    let mut r = Renderer::headless().unwrap();
    assert!(
        r.set_definitions(&pm_render::all_builtin_definitions())
            .unwrap()
            .is_empty()
    );
    let obj = object3d(
        "object",
        "sphere",
        json!({"radius":1,"segments":16}),
        "#7298C4",
        64,
        32,
        1.0,
    );
    let overlay = layer(
        "overlay",
        "solid",
        json!({"color":"#E5A56299","w":25,"h":9}),
        3.0,
        3.0,
        1.0,
    );
    let mut d = project(
        64,
        32,
        1.0,
        solid_fill("#182132"),
        vec![overlay, obj, camera("camera", [0.0, 0.0, 5.0], false, 1.0)],
    );
    let req = RenderRequest::sized(Size::new(64, 32), Precision::Float32);
    for depth in [8, 16, 32] {
        d["colorDepth"] = json!(depth);
        let frame = r
            .render(
                &load(&d),
                "Ptest-root",
                support::ticks(0.0),
                &req,
                &MapResolver::default(),
            )
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let pixels = r.read_rgba(&frame).unwrap();
        assert!(pixels.iter().all(|v| v.is_finite()));
        let colors: std::collections::HashSet<_> = pixels
            .chunks_exact(4)
            .map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()])
            .collect();
        assert!(
            colors.len() > 8,
            "the sphere and overlay draw, depth {depth}"
        );
    }
}

/// Half-float working storage holds the nearest half of each computed value,
/// for a plain draw and for a blend op's result (Metal's render-target
/// conversion truncates: 9/255 would store below its nearest half).
#[test]
fn half_float_stores_round_to_nearest() {
    let nearest = |v: f32| pm_gpu::readback::half_to_f32(pm_gpu::readback::f32_to_half(v));
    let v = 9.0f32 / 255.0;
    assert!(nearest(v) > v, "the fixture value rounds up");
    for blend in [None, Some("screen")] {
        let mut l = layer(
            "a",
            "solid",
            json!({"color":"#090909","w":8,"h":8}),
            4.0,
            4.0,
            1.0,
        );
        if let Some(mode) = blend {
            l["blend"] = json!(mode);
        }
        let mut value = project(8, 8, 1.0, solid_fill("#000000"), vec![l]);
        value["colorDepth"] = json!(16);
        let doc = load(&value);
        let request = RenderRequest::sized(Size::new(8, 8), Precision::Float16);
        let mut renderer = Renderer::headless().unwrap();
        let frame = renderer
            .render(&doc, "Ptest-root", support::ticks(0.0), &request, &MapResolver::default())
            .unwrap();
        let pixels = renderer.read_rgba(&frame).unwrap();
        // Screen over black is the source itself: the op computes 9/255.
        assert_eq!(pixels[(4 * 8 + 4) * 4], nearest(v), "blend {blend:?}");
    }
}
