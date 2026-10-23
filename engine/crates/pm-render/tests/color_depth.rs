//! Project depth must control all intermediate writes, including exports
//! whose caller still requests the former automatic float32 promotion.
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
        (8, wgpu::TextureFormat::Rgba8Unorm, 4),
        (32, wgpu::TextureFormat::Rgba32Float, 16),
        (16, wgpu::TextureFormat::Rgba16Float, 8),
        (8, wgpu::TextureFormat::Rgba8Unorm, 4),
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
        if depth == 8 {
            assert!(
                first
                    .iter()
                    .all(|v| (v * 255.0 - (v * 255.0).round()).abs() < 0.0001)
            );
        }
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
                8 => Precision::Unorm8,
                32 => Precision::Float32,
                _ => Precision::Float16,
            })
        );
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
