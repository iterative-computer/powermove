//! Compatibility rasters (`pm_render::compat`; plan section 20, owner
//! amendments 23 and 24): with a `raster` override in the session, layers
//! today's compositor rasterizes through `PM.raster` draw the producer's
//! answer on the quad it describes, pass through to the engine, or hold
//! the frame until the answer arrives.

mod support;

use std::sync::{Arc, Mutex};

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::compat::{CompatAnswer, CompatFormat, CompatRaster, CompatRasters, CompatSource};
use pm_render::{MapResolver, RenderError, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const W: u32 = 256;
const H: u32 = 256;

struct Producer {
    producing: bool,
    answer: CompatAnswer,
    asked: Mutex<Vec<(String, String, f64, f64)>>,
}

impl Producer {
    fn new(producing: bool, answer: CompatAnswer) -> Arc<Producer> {
        Arc::new(Producer {
            producing,
            answer,
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl CompatRasters for Producer {
    fn producing(&self, source: CompatSource) -> bool {
        self.producing && source == CompatSource::RasterOverride
    }
    fn answer(
        &self,
        composition: &str,
        layer: &str,
        time: f64,
        scale: f64,
        _source: CompatSource,
    ) -> CompatAnswer {
        self.asked
            .lock()
            .unwrap()
            .push((composition.into(), layer.into(), time, scale));
        self.answer.clone()
    }
}

/// A 100 × 60 white rectangle shape and a 2× scaled solid, centered.
fn doc(shape_scale: f64) -> Value {
    let mut shape = layer(
        "box",
        "shape",
        json!({ "shape": "rect", "w": 100, "h": 60, "color": "#FFFFFF", "stroke": 0 }),
        128.0,
        128.0,
        2.0,
    );
    set(&mut shape, "scale.x", json!(shape_scale));
    set(&mut shape, "scale.y", json!(shape_scale));
    project(W, H, 2.0, no_fill(), vec![shape])
}

fn render(renderer: &mut Renderer, doc: &Value, time: f64) -> Result<Vec<f32>, RenderError> {
    let doc = load(doc);
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let frame = renderer.render(
        &doc,
        "Ptest-root",
        ticks(time),
        &request,
        &MapResolver::default(),
    )?;
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    Ok(renderer.read_rgba(&frame).unwrap())
}

fn px(rgba: &[f32], x: u32, y: u32) -> [f32; 4] {
    let i = ((y * W + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

/// A raster of `w` × `h` pixels of one premultiplied 8-bit color.
fn raster(w: u32, h: u32, color: [u8; 4], size: [f64; 2], anchor: [f64; 2]) -> Arc<CompatRaster> {
    Arc::new(CompatRaster {
        width: w,
        height: h,
        format: CompatFormat::Rgba8Premultiplied,
        anchor,
        size,
        uv: [0.0, 0.0, 1.0, 1.0],
        data: Arc::new(color.repeat((w * h) as usize)),
    })
}

#[test]
fn without_a_producer_layers_are_the_engine_s_own_and_nobody_is_asked() {
    let mut renderer = Renderer::headless().unwrap();
    let native = render(&mut renderer, &doc(100.0), 0.0).unwrap();
    let idle = Producer::new(false, CompatAnswer::Missing);
    renderer.set_compat_rasters(Some(idle.clone()));
    assert_eq!(render(&mut renderer, &doc(100.0), 0.0).unwrap(), native);
    assert!(idle.asked.lock().unwrap().is_empty());
}

#[test]
fn a_pass_through_is_drawn_exactly_as_the_engine_draws_it() {
    let mut renderer = Renderer::headless().unwrap();
    let native = render(&mut renderer, &doc(100.0), 0.5).unwrap();
    let pass = Producer::new(true, CompatAnswer::Native);
    renderer.set_compat_rasters(Some(pass.clone()));
    assert_eq!(render(&mut renderer, &doc(100.0), 0.5).unwrap(), native);
    let asked = pass.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        (asked[0].0.as_str(), asked[0].1.as_str(), asked[0].2),
        ("Ptest-root", "box", 0.5)
    );
    assert!(
        (asked[0].3 - 1.0).abs() < 1e-12,
        "raster density {}",
        asked[0].3
    );
}

#[test]
fn a_missing_answer_holds_the_frame_and_names_what_it_needs_at_today_s_density() {
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_compat_rasters(Some(Producer::new(true, CompatAnswer::Missing)));
    match render(&mut renderer, &doc(250.0), 1.0 / 30.0) {
        Err(RenderError::CompatPending(needs)) => {
            assert_eq!(needs.len(), 1);
            let need = &needs[0];
            assert_eq!(need.layer.as_str(), "box");
            assert_eq!(need.composition.as_str(), "Ptest-root");
            assert_eq!(need.time.get(), 23_520_000);
            assert_eq!(need.source, CompatSource::RasterOverride);
            // continuousRasterScale of a 250% layer at scale 1.
            assert!((need.scale - 2.5).abs() < 1e-9, "{}", need.scale);
        }
        other => panic!("expected CompatPending, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn a_producer_s_raster_replaces_the_layer_on_its_own_quad_through_the_layer_s_transform() {
    let mut renderer = Renderer::headless().unwrap();
    // A red raster of 100 × 40 layer pixels anchored at its center, drawn
    // at 2 pixels per layer pixel: the quad is 100 × 40 around the layer's
    // position (scale 100%), where the white 100 × 60 rectangle was.
    let red = raster(200, 80, [255, 0, 0, 255], [100.0, 40.0], [50.0, 20.0]);
    renderer.set_compat_rasters(Some(Producer::new(true, CompatAnswer::Raster(red))));
    let out = render(&mut renderer, &doc(100.0), 0.0).unwrap();
    let center = px(&out, 128, 128);
    assert!(
        center[0] > 0.99 && center[1] < 0.01 && center[2] < 0.01 && center[3] > 0.99,
        "{center:?}"
    );
    // Inside the rectangle, outside the raster's quad: nothing.
    assert_eq!(px(&out, 128, 155)[3], 0.0);
    // The quad's edges: 100 wide, 40 high, centered on (128, 128).
    assert!(px(&out, 79, 128)[3] > 0.99 && px(&out, 77, 128)[3] == 0.0);
    assert!(px(&out, 176, 128)[3] > 0.99 && px(&out, 178, 128)[3] == 0.0);
    assert!(px(&out, 128, 109)[3] > 0.99 && px(&out, 128, 107)[3] == 0.0);
}

#[test]
fn rasters_are_premultiplied_canvas_values_with_the_layer_s_opacity_applied() {
    let mut renderer = Renderer::headless().unwrap();
    // Half-transparent premultiplied white, as a canvas stores it.
    let half = raster(10, 10, [128, 128, 128, 128], [100.0, 60.0], [50.0, 30.0]);
    renderer.set_compat_rasters(Some(Producer::new(true, CompatAnswer::Raster(half))));
    let mut d = doc(100.0);
    set(&mut d["layers"][0], "opacity", json!(50));
    let out = render(&mut renderer, &d, 0.0).unwrap();
    let center = px(&out, 128, 128);
    assert!((center[3] - 0.5 * 128.0 / 255.0).abs() < 2e-3, "{center:?}");
}
