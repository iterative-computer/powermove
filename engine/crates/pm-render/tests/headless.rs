//! The headless render API (deliverable 5) on generated documents: the
//! same frame twice is bit-identical (section 2, "Determinism"), a region
//! of interest equals the same pixels of the full frame, the depth and id
//! surface reports layers and 3D depth (contract 2), and the capability
//! check lists what the renderer cannot draw (section 21 routing).

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::{PixelRect, Size};
use pm_render::{Feature, MapResolver, RenderRequest, Renderer, unsupported};
use serde_json::{Value, json};
use support::doc::*;
use support::{repo_root, ticks};

const W: u32 = 320;
const H: u32 = 180;

/// A composition using every feature the renderer supports.
fn everything() -> (Value, MapResolver) {
    let dur = 2.0;
    let mut layers = Vec::new();
    // Top to bottom.
    let mut over = layer("over", "solid", json!({ "color": "#E8E2CF", "w": 40, "h": 40 }), 10.0, 120.0, dur);
    set(&mut over, "opacity", json!(70));
    set(&mut over, "rotation", keys(json!(0), &[(0.0, json!(0)), (2.0, json!(90))]));
    layers.push(over);
    layers.push(object3d("obj", "sphere", json!({ "radius": 1, "segments": 32 }), "#4C8DFF", W, H, dur));
    layers.push(sun("sun", [-4.0, 6.0, 4.0], false, dur));
    layers.push(camera("cam", [0.0, 0.0, 5.0], false, dur));
    let mut image = layer("img", "image", json!({ "asset": "still", "fit": "contain", "w": 80, "h": 45 }), 250.0, 40.0, dur);
    set(&mut image, "rotation", json!(17));
    layers.push(image);
    let mut video = layer("vid", "video", json!({ "asset": "clip", "fit": "cover", "w": 120, "h": 68, "trim": 0, "speed": 1 }), 80.0, 50.0, dur);
    set(&mut video, "rotation", json!(-8));
    set(&mut video, "opacity", json!(85));
    layers.push(video);
    let mut a = layer("ga", "solid", json!({ "color": "#FF6B1A", "w": 60, "h": 60 }), 200.0, 100.0, dur);
    a["group"] = json!("grp");
    let mut b = layer("gb", "solid", json!({ "color": "#3FCF8E", "w": 60, "h": 60 }), 230.0, 120.0, dur);
    b["group"] = json!("grp");
    layers.push(a);
    layers.push(b);
    let mut group = layer("grp", "group", json!({}), 0.0, 0.0, dur);
    set(&mut group, "opacity", json!(60));
    layers.push(group);
    let mut pre = layer("pre", "precomp", json!({ "comp": "inner", "w": 160, "h": 90 }), 150.0, 10.0, dur);
    set(&mut pre, "scale.x", json!(75));
    set(&mut pre, "scale.y", json!(75));
    layers.push(pre);
    layers.push(layer("back", "solid", json!({ "color": "#202436", "w": 200, "h": 180 }), 0.0, 0.0, dur));
    let mut doc = project(W, H, dur, solid_fill("#14161C"), layers);
    composition(
        &mut doc,
        "inner",
        160,
        90,
        dur,
        vec![
            layer("in-img", "image", json!({ "asset": "still", "fit": "stretch", "w": 160, "h": 90 }), 80.0, 45.0, dur),
            layer("in-solid", "solid", json!({ "color": "#A73366", "w": 160, "h": 90 }), 0.0, 0.0, dur),
        ],
    );
    asset(&mut doc, "still", "image", "png", 160, 90, 0.0);
    asset(&mut doc, "clip", "video", "mp4", 320, 180, 2.0);
    let fixtures = repo_root().join("apps/desktop/e2e/fixtures");
    let mut resolver = MapResolver::default();
    resolver.paths.insert("still".into(), fixtures.join("still-alpha.png"));
    resolver.paths.insert("clip".into(), fixtures.join("h264-aac.mp4"));
    (doc, resolver)
}

fn render_bytes(
    renderer: &mut Renderer,
    doc: &pm_model::Document,
    t: f64,
    request: &RenderRequest,
    resolver: &MapResolver,
) -> (Vec<u8>, Option<Vec<u8>>, Vec<String>) {
    let frame = renderer
        .render(doc, "Ptest-root", ticks(t), request, resolver)
        .expect("render");
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    let color = pm_gpu::readback::read_raw(renderer.device(), renderer.queue(), &frame.color.texture).unwrap();
    let ids = frame
        .depth_id
        .as_ref()
        .map(|t| pm_gpu::readback::read_raw(renderer.device(), renderer.queue(), &t.texture).unwrap());
    (color, ids, frame.legend)
}

#[test]
fn every_feature_is_supported_and_frames_are_deterministic() {
    let (doc, resolver) = everything();
    let doc = load(&doc);
    assert_eq!(unsupported(&doc, "Ptest-root"), vec![], "the composition is within the renderer");
    let mut first = Renderer::headless().unwrap();
    let mut second = Renderer::headless().unwrap();
    for precision in [Precision::Float16, Precision::Float32] {
        let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, precision);
        request.depth_id = true;
        for t in [0.0, 0.73, 1.5] {
            let a = render_bytes(&mut first, &doc, t, &request, &resolver);
            let b = render_bytes(&mut first, &doc, t, &request, &resolver);
            let c = render_bytes(&mut second, &doc, t, &request, &resolver);
            assert!(a.0 == b.0 && a.1 == b.1, "{precision:?} t={t}: the same renderer twice differs");
            assert!(a.0 == c.0 && a.1 == c.1, "{precision:?} t={t}: two renderers differ");
            assert!(a.0.iter().any(|&v| v != 0), "the frame drew something");
        }
    }
}

#[test]
fn regions_equal_the_same_pixels_of_the_full_frame() {
    let (doc, resolver) = everything();
    let doc = load(&doc);
    let mut r = Renderer::headless().unwrap();
    for precision in [Precision::Float16, Precision::Float32] {
        let mut full = RenderRequest::at_scale(Size::new(W, H), 1.0, precision);
        full.depth_id = true;
        let (whole, whole_ids, _) = render_bytes(&mut r, &doc, 0.6, &full, &resolver);
        let whole_ids = whole_ids.unwrap();
        let bpp = if precision == Precision::Float16 { 8 } else { 16 };
        for rect in [
            PixelRect { x: 0, y: 0, width: W, height: H },
            PixelRect { x: 101, y: 37, width: 150, height: 100 },
            PixelRect { x: 1, y: 179, width: 319, height: 1 },
            PixelRect { x: 140, y: 60, width: 60, height: 70 },
        ] {
            let mut request = full;
            request.region = Some(rect);
            let (part, part_ids, _) = render_bytes(&mut r, &doc, 0.6, &request, &resolver);
            let part_ids = part_ids.unwrap();
            // The depth and id surface (rgba16float) follows the region too.
            for y in 0..rect.height as usize {
                let row = rect.width as usize * 8;
                let src = ((rect.y as usize + y) * W as usize + rect.x as usize) * 8;
                assert!(
                    part_ids[y * row..(y + 1) * row] == whole_ids[src..src + row],
                    "{precision:?} {rect:?}: id row {y} differs from the full frame"
                );
            }
            let mut bad = Vec::new();
            for y in 0..rect.height as usize {
                for x in 0..rect.width as usize {
                    let a = (y * rect.width as usize + x) * bpp;
                    let b = ((rect.y as usize + y) * W as usize + rect.x as usize + x) * bpp;
                    if part[a..a + bpp] != whole[b..b + bpp] {
                        bad.push((rect.x as usize + x, rect.y as usize + y));
                    }
                }
            }
            if !bad.is_empty() {
                eprintln!("{precision:?} {rect:?}: {} pixels differ, first {:?}", bad.len(), &bad[..bad.len().min(12)]);
            }
            for y in 0..rect.height as usize {
                let row = rect.width as usize * bpp;
                let src = ((rect.y as usize + y) * W as usize + rect.x as usize) * bpp;
                assert!(
                    part[y * row..(y + 1) * row] == whole[src..src + row],
                    "{precision:?} {rect:?}: row {y} differs from the full frame"
                );
            }
        }
    }
}

#[test]
fn depth_and_id_surface() {
    let dur = 2.0;
    let mut doc = project(W, H, dur, solid_fill("#000000"), vec![
        object3d("obj", "sphere", json!({ "radius": 1, "segments": 64 }), "#FFFFFF", W, H, dur),
        sun("sun", [-4.0, 6.0, 4.0], false, dur),
        camera("cam", [0.0, 0.0, 5.0], false, dur),
        layer("left", "solid", json!({ "color": "#FF0000", "w": 60, "h": 180 }), 0.0, 0.0, dur),
    ]);
    doc["layers"][3]["p"]["opacity"] = ch(json!(100));
    let doc = load(&doc);
    let mut r = Renderer::headless().unwrap();
    let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    request.depth_id = true;
    let frame = r.render(&doc, "Ptest-root", ticks(0.0), &request, &MapResolver::default()).unwrap();
    let surface = r.read_depth_id(&frame).unwrap().unwrap();
    let at = |x: usize, y: usize| {
        let i = (y * W as usize + x) * 4;
        (surface[i], surface[i + 1], surface[i + 2], surface[i + 3])
    };
    let index = |id: &str| frame.legend.iter().position(|l| l == id).unwrap() as f32;
    // The sphere's center is on the camera axis: depth 5 − 1.
    let (d, lo, hi, a) = at(160, 90);
    assert_eq!((lo, hi, a), (index("obj"), 0.0, 1.0));
    assert!((d - 4.0).abs() < 4e-3, "center depth {d}");
    // Off the sphere's silhouette but inside the frame: nothing.
    assert_eq!(at(310, 10).3, 0.0);
    assert!(at(310, 10).0.is_infinite());
    // The 2D solid on the left: its id, no depth.
    let (d, lo, _, a) = at(30, 90);
    assert_eq!((lo, a), (index("left"), 1.0));
    assert!(d.is_infinite());
    // Depth grows toward the silhouette (a sphere seen head-on).
    assert!(at(185, 90).0 > at(160, 90).0);
}

fn variant(edit: impl FnOnce(&mut Value)) -> Vec<Feature> {
    let (mut doc, _) = everything();
    edit(&mut doc);
    unsupported(&load(&doc), "Ptest-root")
        .into_iter()
        .map(|u| u.feature)
        .collect()
}

#[test]
fn capability_check_lists_what_the_renderer_cannot_draw() {
    let layer_at = |doc: &mut Value, id: &str| -> usize {
        doc["layers"].as_array().unwrap().iter().position(|l| l["id"] == id).unwrap()
    };
    // Layer types and effects.
    let f = variant(|d| d["layers"].as_array_mut().unwrap().push(layer("t", "text", json!({ "text": "x" }), 0.0, 0.0, 2.0)));
    assert!(f.contains(&Feature::LayerType("text".into())), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "back");
        d["layers"][i]["blend"] = json!("multiply");
    });
    assert!(f.contains(&Feature::BlendMode("multiply".into())), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "back");
        d["layers"][i]["fx"] = json!([{ "id": "e1", "type": "blur", "on": true, "p": {} }]);
    });
    assert!(f.contains(&Feature::Effects), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "back");
        d["layers"][i]["mblur"] = json!(true);
    });
    assert!(f.contains(&Feature::MotionBlur), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "back");
        d["layers"][i]["threeD"] = json!(true);
    });
    assert!(f.contains(&Feature::TwoAndAHalfD), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "back");
        d["layers"][i]["masks"] = json!([{ "id": "m1", "on": true }]);
    });
    assert!(f.contains(&Feature::Masks), "{f:?}");
    let f = variant(|d| d["color"] = json!({ "mode": "standard", "linearBlending": true }));
    assert!(f.contains(&Feature::ProjectColor), "{f:?}");
    // 3D beyond the Phase 2 primitive.
    let scene = |f: &[Feature], what: &str| f.iter().any(|x| matches!(x, Feature::Scene3d(s) if s.contains(what)));
    let f = variant(|d| {
        let i = layer_at(d, "cam");
        d["layers"][i]["d"]["data"]["environment"]["shadows"] = json!(true);
        let i = layer_at(d, "sun");
        d["layers"][i]["d"]["data"]["light"]["castShadow"] = json!(true);
    });
    assert!(scene(&f, "shadows"), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "sun");
        d["layers"][i]["d"]["data"]["light"]["type"] = json!("point");
    });
    assert!(scene(&f, "point lights"), "{f:?}");
    let f = variant(|d| {
        d["layers"].as_array_mut().unwrap().push(object3d("obj2", "box", json!({}), "#FFFFFF", W, H, 2.0));
    });
    assert!(scene(&f, "more than one 3D object"), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["material"]["p"]["opacity"] = ch(json!(0.5));
    });
    assert!(scene(&f, "transparent materials"), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["source"] = json!({ "primitive": "capsule" });
    });
    assert!(scene(&f, "capsule"), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["source"] = json!({ "assetId": "model" });
    });
    assert!(scene(&f, "imported models"), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["material"]["maps"] = json!({ "color": "tex" });
    });
    assert!(scene(&f, "texture maps"), "{f:?}");
    // No camera layer: the studio environment has shadows on and the sun
    // layer casts by default.
    let f = variant(|d| {
        let i = layer_at(d, "cam");
        d["layers"].as_array_mut().unwrap().remove(i);
        let i = layer_at(d, "sun");
        d["layers"][i]["d"]["data"]["light"]["castShadow"] = json!(true);
    });
    assert!(scene(&f, "shadows"), "{f:?}");
    // Nested compositions are checked too.
    let f = variant(|d| d["comps"]["inner"]["layers"].as_array_mut().unwrap().push(layer("s", "shape", json!({}), 0.0, 0.0, 2.0)));
    assert!(f.contains(&Feature::LayerType("shape".into())), "{f:?}");
}

#[test]
fn requests_are_validated() {
    let (doc, resolver) = everything();
    let doc = load(&doc);
    let mut r = Renderer::headless().unwrap();
    let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    request.region = Some(PixelRect { x: 300, y: 0, width: 40, height: 10 });
    assert!(r.render(&doc, "Ptest-root", ticks(0.0), &request, &resolver).is_err());
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    assert!(r.render(&doc, "nope", ticks(0.0), &request, &resolver).is_err());
}
