//! Compatibility rasters (`pm_render::compat`; plan section 20, owner
//! amendments 23 and 24): with a `raster` override in the session, layers
//! today's compositor rasterizes through `PM.raster` draw the producer's
//! answer on the quad it describes, pass through to the engine, or hold
//! the frame until the answer arrives.

mod support;

use std::sync::{Arc, Mutex};

use pm_protocol::color::Precision;
use pm_protocol::common::{PixelRect, Size};
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
    crops: Mutex<Vec<Option<PixelRect>>>,
}

impl Producer {
    fn new(producing: bool, answer: CompatAnswer) -> Arc<Producer> {
        Arc::new(Producer {
            producing,
            answer,
            asked: Mutex::new(Vec::new()),
            crops: Mutex::new(Vec::new()),
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
        crop: Option<PixelRect>,
    ) -> CompatAnswer {
        self.crops.lock().unwrap().push(crop);
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

/// One shape layer at `(x, y)` of a 256 × 256 composition.
fn shape_doc(id: &str, d: Value, x: f64, y: f64) -> Value {
    project(W, H, 2.0, no_fill(), vec![layer(id, "shape", d, x, y, 2.0)])
}

/// A viewer frame: the composition at `zoom` with the visible `region`.
fn viewer(
    renderer: &mut Renderer,
    doc: &Value,
    zoom: f64,
    region: PixelRect,
    export: bool,
) -> Result<(), RenderError> {
    let doc = load(doc);
    let mut request = RenderRequest::at_scale(Size::new(W, H), zoom, Precision::Float32);
    request.region = Some(region);
    request.export = export;
    renderer
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .map(|_| ())
}

#[test]
fn viewer_frames_ask_for_today_s_preview_crop_window() {
    // A 200 × 100 rect (not a plain fill: half-transparent) at the centre
    // (output 2048 × 2048 at 8×, the rect spans x 224..1824), viewed
    // through a 512 × 512 window over its left edge.
    let d = json!({ "shape": "rect", "w": 200, "h": 100, "color": "#FFFFFF80", "stroke": 0 });
    let doc = shape_doc("box", d, 128.0, 128.0);
    let mut renderer = Renderer::headless().unwrap();
    let producer = Producer::new(true, CompatAnswer::Missing);
    renderer.set_compat_rasters(Some(producer.clone()));
    let region = PixelRect {
        x: 0,
        y: 768,
        width: 512,
        height: 512,
    };
    match viewer(&mut renderer, &doc, 8.0, region, false) {
        Err(RenderError::CompatPending(needs)) => {
            assert_eq!(needs.len(), 1);
            // Today's previewShapeRaster for the same inputs
            // (`[8, 0, 0, 8, 1024, 256]` in a 512 × 512 canvas; raster
            // 1664 × 864): `{ x: 0, y: 0, width: 512, height: 768 }`.
            assert_eq!(
                needs[0].crop,
                Some(PixelRect {
                    x: 0,
                    y: 0,
                    width: 512,
                    height: 768
                })
            );
            assert!((needs[0].scale - 8.0).abs() < 1e-12);
        }
        other => panic!("expected the crop need, got {:?}", other.err()),
    }
    // The producer was asked with the same window.
    assert_eq!(
        producer.crops.lock().unwrap().last().copied().flatten(),
        Some(PixelRect {
            x: 0,
            y: 0,
            width: 512,
            height: 768
        })
    );
    // An export of the same frame never crops.
    producer.crops.lock().unwrap().clear();
    let Err(RenderError::CompatPending(needs)) = viewer(&mut renderer, &doc, 8.0, region, true)
    else {
        panic!("expected a need");
    };
    assert_eq!(needs[0].crop, None);
    assert_eq!(producer.crops.lock().unwrap().as_slice(), &[None]);
}

#[test]
fn the_open_canonical_composition_gets_the_same_preview_crop_as_stored_projects() {
    let d = json!({ "shape": "rect", "w": 200, "h": 100, "color": "#FFFFFF80", "stroke": 0 });
    let mut stored = shape_doc("box", d, 128.0, 128.0);
    stored["compId"] = json!("Ptest-root");
    stored["comps"] = json!({});
    let canonical =
        pm_model::canonical::canonical_project(pm_model::json::parse(&stored.to_string()).unwrap());
    let canonical: Value =
        serde_json::from_str(&pm_model::json::to_canonical_string(&canonical)).unwrap();
    let mut renderer = Renderer::headless().unwrap();
    let producer = Producer::new(true, CompatAnswer::Missing);
    renderer.set_compat_rasters(Some(producer));
    let region = PixelRect {
        x: 0,
        y: 768,
        width: 512,
        height: 512,
    };
    for doc in [&stored, &canonical] {
        let Err(RenderError::CompatPending(needs)) = viewer(&mut renderer, doc, 8.0, region, false)
        else {
            panic!("expected a need");
        };
        assert_eq!(
            needs[0].crop,
            Some(PixelRect {
                x: 0,
                y: 0,
                width: 512,
                height: 768
            })
        );
    }
}

#[test]
fn a_viewport_that_misses_the_shape_or_sits_in_its_plain_fill_does_not_ask() {
    let mut renderer = Renderer::headless().unwrap();
    let producer = Producer::new(true, CompatAnswer::Missing);
    renderer.set_compat_rasters(Some(producer.clone()));
    // Off to the right: nothing to draw, nobody asked.
    let d = json!({ "shape": "rect", "w": 40, "h": 40, "color": "#FF0000", "stroke": 0 });
    let away = PixelRect {
        x: 1800,
        y: 0,
        width: 200,
        height: 200,
    };
    viewer(
        &mut renderer,
        &shape_doc("box", d.clone(), 40.0, 40.0),
        8.0,
        away,
        false,
    )
    .unwrap();
    assert!(producer.asked.lock().unwrap().is_empty());
    // Inside a plain #rrggbb rect's fill: today fills the viewport with
    // its color without calling PM.raster; the engine draws it natively.
    let big = json!({ "shape": "rect", "w": 200, "h": 200, "color": "#FF0000", "stroke": 0 });
    let inside = PixelRect {
        x: 1024 - 64,
        y: 1024 - 64,
        width: 128,
        height: 128,
    };
    viewer(
        &mut renderer,
        &shape_doc("box", big, 128.0, 128.0),
        8.0,
        inside,
        false,
    )
    .unwrap();
    assert!(producer.asked.lock().unwrap().is_empty());
}

#[test]
fn rasters_of_any_width_keep_their_rows() {
    // 200 × 132 (800 bytes a row: not a multiple of 256), one opaque red
    // column at x = 50 on transparent rows; drawn at one pixel per layer
    // pixel, centered on (128, 128) at scale 1: the column at x = 78 in
    // every row, nothing beside it.
    let (w, h) = (200u32, 132u32);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let i = ((y * w + 50) * 4) as usize;
        data[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
    }
    let column = Arc::new(CompatRaster {
        width: w,
        height: h,
        format: CompatFormat::Rgba8Premultiplied,
        anchor: [100.0, 66.0],
        size: [200.0, 132.0],
        uv: [0.0, 0.0, 1.0, 1.0],
        data: Arc::new(data),
    });
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_compat_rasters(Some(Producer::new(true, CompatAnswer::Raster(column))));
    let doc = load(&doc(100.0));
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .unwrap();
    let out = renderer.read_rgba(&frame).unwrap();
    for y in [63u32, 64, 100, 128, 160, 193] {
        let on = px(&out, 78, y);
        assert!(on[0] > 0.99 && on[3] > 0.99, "row {y}: column {on:?}");
        for x in [70u32, 77, 79, 90, 150] {
            assert_eq!(px(&out, x, y)[3], 0.0, "row {y}, x {x}");
        }
    }
}

#[test]
fn a_text_layer_s_raster_keeps_its_rows() {
    for precision in [Precision::Float32, Precision::Float16] {
        text_rows(precision);
    }
}

fn text_rows(precision: Precision) {
    let (w, h) = (200u32, 132u32);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let i = ((y * w + 50) * 4) as usize;
        data[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
    }
    let column = Arc::new(CompatRaster {
        width: w,
        height: h,
        format: CompatFormat::Rgba8Premultiplied,
        anchor: [100.0, 66.0],
        size: [200.0, 132.0],
        uv: [0.0, 0.0, 1.0, 1.0],
        data: Arc::new(data),
    });
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_compat_rasters(Some(Producer::new(true, CompatAnswer::Raster(column))));
    let text = layer(
        "title",
        "text",
        json!({ "text": "Shifted", "font": "Helvetica", "size": 36, "weight": 700, "color": "#FFFFFF" }),
        128.0,
        128.0,
        2.0,
    );
    let doc = load(&project(W, H, 2.0, no_fill(), vec![text]));
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, precision);
    let frame = renderer
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .unwrap();
    let out = renderer.read_rgba(&frame).unwrap();
    for y in [63u32, 64, 100, 128, 160, 193] {
        let on = px(&out, 78, y);
        assert!(
            on[0] > 0.99 && on[3] > 0.99,
            "{precision:?} row {y}: column {on:?}"
        );
        for x in [70u32, 77, 79, 90, 150] {
            assert_eq!(px(&out, x, y)[3], 0.0, "{precision:?} row {y}, x {x}");
        }
    }
}

/// Bigrender round 2: a viewer waiting for one frame's answers asks for the
/// next frames' from their plans alone (`Renderer::compat_needs`): the same
/// needs the frame itself names when it is drawn, at its own time, with
/// nothing recorded.
#[test]
fn a_frame_s_needs_from_its_plan_alone_are_the_ones_its_render_names() {
    let mut renderer = Renderer::headless().unwrap();
    renderer.set_compat_rasters(Some(Producer::new(true, CompatAnswer::Missing)));
    let doc_value = doc(250.0);
    let document = load(&doc_value);
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    for t in [1.0 / 30.0, 0.5, 1.25] {
        let planned = renderer.compat_needs(
            &document,
            "Ptest-root",
            ticks(t),
            &request,
            &MapResolver::default(),
        );
        let Err(RenderError::CompatPending(drawn)) = render(&mut renderer, &doc_value, t) else {
            panic!("expected CompatPending at {t}");
        };
        assert_eq!(planned, drawn, "at {t}");
        assert_eq!(planned[0].time.get(), ticks(t).get());
    }
    // A pass-through producer: nothing to ask.
    renderer.set_compat_rasters(Some(Producer::new(true, CompatAnswer::Native)));
    assert!(renderer
        .compat_needs(&document, "Ptest-root", ticks(0.5), &request, &MapResolver::default())
        .is_empty());
    // No producer: nothing to ask either.
    renderer.set_compat_rasters(None);
    assert!(renderer
        .compat_needs(&document, "Ptest-root", ticks(0.5), &request, &MapResolver::default())
        .is_empty());
}
