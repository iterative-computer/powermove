//! Compositing ops (blend modes, masks) take their uniforms from the frame's
//! uniform pool: a steady frame allocates no GPU buffer per op, however many
//! ops it runs.
mod support;
use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

fn scene(layers: usize) -> Value {
    let mut all = vec![layer(
        "bg",
        "solid",
        json!({"w":128,"h":96,"color":"#336699"}),
        64.,
        48.,
        1.,
    )];
    for i in 0..layers {
        let mut l = layer(
            &format!("l{i}"),
            "solid",
            json!({"w":30,"h":20,"color":"#F0A040C0"}),
            20. + 9. * i as f64,
            30. + 5. * i as f64,
            1.,
        );
        l["blend"] = json!(if i % 2 == 0 { "multiply" } else { "screen" });
        l["masks"] = json!([{"id":"m","shape":"rect","mode":"add","on":true,"p":{
            "x":ch(json!(2)),"y":ch(json!(2)),"w":ch(json!(20)),"h":ch(json!(14)),
            "rotation":ch(json!(0)),"feather":ch(json!(2))}}]);
        all.push(l);
    }
    project(128, 96, 1., no_fill(), all)
}

/// The frame's pixels and the GPU buffers its second render allocated.
fn steady(r: &mut Renderer, v: &Value) -> (Vec<f32>, u64) {
    let doc = load(v);
    let req = RenderRequest::at_scale(Size::new(128, 96), 1., Precision::Float16);
    let mut draw = || {
        let f = r
            .render(&doc, "Ptest-root", ticks(0.25), &req, &MapResolver::default())
            .unwrap();
        assert!(f.failures.is_empty(), "{:?}", f.failures);
        f
    };
    draw();
    let scope = pm_perf::Scope::begin();
    let f = draw();
    let buffers = scope.finish().get(pm_perf::Counter::BufferAllocs);
    (r.read_rgba(&f).unwrap(), buffers)
}

#[test]
fn compositing_ops_allocate_no_buffer_per_op() {
    let mut r = Renderer::headless().unwrap();
    let (few_pixels, few) = steady(&mut r, &scene(2));
    let (many_pixels, many) = steady(&mut r, &scene(8));
    assert_eq!(few, many, "buffers allocated grow with the ops a frame runs");
    // The ops drew (a blend of every layer over the background).
    assert_ne!(few_pixels, many_pixels);
}
