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
    let mut over = layer(
        "over",
        "solid",
        json!({ "color": "#E8E2CF", "w": 40, "h": 40 }),
        10.0,
        120.0,
        dur,
    );
    set(&mut over, "opacity", json!(70));
    set(
        &mut over,
        "rotation",
        keys(json!(0), &[(0.0, json!(0)), (2.0, json!(90))]),
    );
    layers.push(over);
    layers.push(object3d(
        "obj",
        "sphere",
        json!({ "radius": 1, "segments": 32 }),
        "#4C8DFF",
        W,
        H,
        dur,
    ));
    layers.push(sun("sun", [-4.0, 6.0, 4.0], false, dur));
    layers.push(camera("cam", [0.0, 0.0, 5.0], false, dur));
    let mut image = layer(
        "img",
        "image",
        json!({ "asset": "still", "fit": "contain", "w": 80, "h": 45 }),
        250.0,
        40.0,
        dur,
    );
    set(&mut image, "rotation", json!(17));
    layers.push(image);
    let mut video = layer(
        "vid",
        "video",
        json!({ "asset": "clip", "fit": "cover", "w": 120, "h": 68, "trim": 0, "speed": 1 }),
        80.0,
        50.0,
        dur,
    );
    set(&mut video, "rotation", json!(-8));
    set(&mut video, "opacity", json!(85));
    layers.push(video);
    let mut a = layer(
        "ga",
        "solid",
        json!({ "color": "#FF6B1A", "w": 60, "h": 60 }),
        200.0,
        100.0,
        dur,
    );
    a["group"] = json!("grp");
    let mut b = layer(
        "gb",
        "solid",
        json!({ "color": "#3FCF8E", "w": 60, "h": 60 }),
        230.0,
        120.0,
        dur,
    );
    b["group"] = json!("grp");
    layers.push(a);
    layers.push(b);
    let mut group = layer("grp", "group", json!({}), 0.0, 0.0, dur);
    set(&mut group, "opacity", json!(60));
    layers.push(group);
    let mut pre = layer(
        "pre",
        "precomp",
        json!({ "comp": "inner", "w": 160, "h": 90 }),
        150.0,
        10.0,
        dur,
    );
    set(&mut pre, "scale.x", json!(75));
    set(&mut pre, "scale.y", json!(75));
    layers.push(pre);
    layers.push(layer(
        "back",
        "solid",
        json!({ "color": "#202436", "w": 200, "h": 180 }),
        0.0,
        0.0,
        dur,
    ));
    let mut doc = project(W, H, dur, solid_fill("#14161C"), layers);
    composition(
        &mut doc,
        "inner",
        160,
        90,
        dur,
        vec![
            layer(
                "in-img",
                "image",
                json!({ "asset": "still", "fit": "stretch", "w": 160, "h": 90 }),
                80.0,
                45.0,
                dur,
            ),
            layer(
                "in-solid",
                "solid",
                json!({ "color": "#A73366", "w": 160, "h": 90 }),
                0.0,
                0.0,
                dur,
            ),
        ],
    );
    asset(&mut doc, "still", "image", "png", 160, 90, 0.0);
    asset(&mut doc, "clip", "video", "mp4", 320, 180, 2.0);
    let fixtures = repo_root().join("apps/desktop/e2e/fixtures");
    let mut resolver = MapResolver::default();
    resolver
        .paths
        .insert("still".into(), fixtures.join("still-alpha.png"));
    resolver
        .paths
        .insert("clip".into(), fixtures.join("h264-aac.mp4"));
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
    let color =
        pm_gpu::readback::read_raw(renderer.device(), renderer.queue(), &frame.color.texture)
            .unwrap();
    let ids = frame.depth_id.as_ref().map(|t| {
        pm_gpu::readback::read_raw(renderer.device(), renderer.queue(), &t.texture).unwrap()
    });
    (color, ids, frame.legend)
}

#[test]
fn every_feature_is_supported_and_frames_are_deterministic() {
    let (doc, resolver) = everything();
    let doc = load(&doc);
    assert_eq!(
        unsupported(&doc, "Ptest-root"),
        vec![],
        "the composition is within the renderer"
    );
    let mut first = Renderer::headless().unwrap();
    let mut second = Renderer::headless().unwrap();
    for precision in [Precision::Float16, Precision::Float32] {
        let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, precision);
        request.depth_id = true;
        for t in [0.0, 0.73, 1.5] {
            let a = render_bytes(&mut first, &doc, t, &request, &resolver);
            let b = render_bytes(&mut first, &doc, t, &request, &resolver);
            let c = render_bytes(&mut second, &doc, t, &request, &resolver);
            assert!(
                a.0 == b.0 && a.1 == b.1,
                "{precision:?} t={t}: the same renderer twice differs"
            );
            assert!(
                a.0 == c.0 && a.1 == c.1,
                "{precision:?} t={t}: two renderers differ"
            );
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
        let bpp = if precision == Precision::Float16 {
            8
        } else {
            16
        };
        for rect in [
            PixelRect {
                x: 0,
                y: 0,
                width: W,
                height: H,
            },
            PixelRect {
                x: 101,
                y: 37,
                width: 150,
                height: 100,
            },
            PixelRect {
                x: 1,
                y: 179,
                width: 319,
                height: 1,
            },
            PixelRect {
                x: 140,
                y: 60,
                width: 60,
                height: 70,
            },
        ] {
            let mut request = full.clone();
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
                eprintln!(
                    "{precision:?} {rect:?}: {} pixels differ, first {:?}",
                    bad.len(),
                    &bad[..bad.len().min(12)]
                );
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
    let mut doc = project(
        W,
        H,
        dur,
        solid_fill("#000000"),
        vec![
            object3d(
                "obj",
                "sphere",
                json!({ "radius": 1, "segments": 64 }),
                "#FFFFFF",
                W,
                H,
                dur,
            ),
            sun("sun", [-4.0, 6.0, 4.0], false, dur),
            camera("cam", [0.0, 0.0, 5.0], false, dur),
            layer(
                "left",
                "solid",
                json!({ "color": "#FF0000", "w": 60, "h": 180 }),
                0.0,
                0.0,
                dur,
            ),
        ],
    );
    doc["layers"][3]["p"]["opacity"] = ch(json!(100));
    let doc = load(&doc);
    let mut r = Renderer::headless().unwrap();
    let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    request.depth_id = true;
    let frame = r
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .unwrap();
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
        doc["layers"]
            .as_array()
            .unwrap()
            .iter()
            .position(|l| l["id"] == id)
            .unwrap()
    };
    // Layer types and effects. Text and captions are drawn (with fonts
    // configured; tests/text_raster.rs and tests/text_corpus.rs).
    let f = variant(|d| {
        d["layers"].as_array_mut().unwrap().push(layer(
            "t",
            "text",
            json!({ "text": "x" }),
            0.0,
            0.0,
            2.0,
        ))
    });
    assert!(
        !f.iter().any(|f| matches!(f, Feature::LayerType(_))),
        "{f:?}"
    );
    // Without fonts the static check lists them; with fonts they are drawn.
    assert!(f.iter().any(|f| matches!(f, Feature::Text(_))), "{f:?}");
    let (mut doc, _) = everything();
    doc["layers"].as_array_mut().unwrap().push(layer(
        "t",
        "text",
        json!({ "text": "x" }),
        0.0,
        0.0,
        2.0,
    ));
    let with_fonts = pm_render::unsupported_with_fonts(&load(&doc), "Ptest-root", None);
    assert!(
        !with_fonts
            .iter()
            .any(|u| matches!(u.feature, Feature::Text(_))),
        "{with_fonts:?}"
    );
    // Blend modes, parametric and bezier path masks, and track mattes are
    // drawn (`pm_render::composite`).
    let f = variant(|d| {
        let i = layer_at(d, "back");
        d["layers"][i]["blend"] = json!("multiply");
        d["layers"][i]["masks"] = json!([{ "id": "m1", "on": true, "shape": "ellipse", "p": {} }]);
        d["layers"][i]["matteSource"] = json!("over");
    });
    assert!(f.is_empty(), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "back");
        d["layers"][i]["fx"] = json!([{ "id": "e1", "type": "blur", "on": true, "p": {} }]);
    });
    assert!(f.contains(&Feature::Effects), "{f:?}");
    // Motion blur and 2.5D planes are drawn for every layer type the
    // renderer draws (docs/engine/motion.md).
    let f = variant(|d| {
        let i = layer_at(d, "back");
        d["layers"][i]["mblur"] = json!(true);
        d["layers"][i]["threeD"] = json!(true);
    });
    assert!(!f.contains(&Feature::MotionBlur), "{f:?}");
    assert!(!f.contains(&Feature::TwoAndAHalfD), "{f:?}");
    let f = variant(|d| {
        let mut shape = layer("sh", "shape", json!({ "shape": "rect" }), 0.0, 0.0, 2.0);
        shape["mblur"] = json!(true);
        shape["threeD"] = json!(true);
        d["layers"].as_array_mut().unwrap().push(shape);
    });
    assert!(!f.contains(&Feature::MotionBlur), "{f:?}");
    assert!(!f.contains(&Feature::TwoAndAHalfD), "{f:?}");
    // Shapes, solids, 3D objects, and cameras inside a 3D group or a
    // blurred group, and the layers of a composition reached through a
    // blurred precomp (or a precomp inside a blurred group), are drawn.
    let (mut doc, _) = everything();
    let mut g = layer("g3", "group", json!({}), 0.0, 0.0, 2.0);
    g["threeD"] = json!(true);
    g["mblur"] = json!(true);
    let mut shape = layer("sh", "shape", json!({ "shape": "rect" }), 0.0, 0.0, 2.0);
    shape["group"] = json!("g3");
    let mut solid = layer("so", "solid", json!({ "color": "#FFFFFF" }), 0.0, 0.0, 2.0);
    solid["group"] = json!("g3");
    let i = layer_at(&mut doc, "obj");
    doc["layers"][i]["group"] = json!("g3");
    let layers = doc["layers"].as_array_mut().unwrap();
    layers.push(shape);
    layers.push(solid);
    layers.push(g);
    let listed = unsupported(&load(&doc), "Ptest-root");
    assert!(
        !listed
            .iter()
            .any(|u| matches!(u.feature, Feature::MotionBlur | Feature::TwoAndAHalfD)),
        "{listed:?}"
    );
    for (blur_precomp, blur_group) in [(true, false), (false, true)] {
        let (mut doc, _) = everything();
        doc["comps"]["inner"]["layers"]
            .as_array_mut()
            .unwrap()
            .push(layer(
                "in-sh",
                "shape",
                json!({ "shape": "rect" }),
                0.0,
                0.0,
                2.0,
            ));
        let i = layer_at(&mut doc, "pre");
        if blur_precomp {
            doc["layers"][i]["mblur"] = json!(true);
        }
        if blur_group {
            let mut g = layer("gb", "group", json!({}), 0.0, 0.0, 2.0);
            g["mblur"] = json!(true);
            doc["layers"][i]["group"] = json!("gb");
            doc["layers"].as_array_mut().unwrap().push(g);
        }
        let listed = unsupported(&load(&doc), "Ptest-root");
        assert!(
            !listed.iter().any(|u| u.feature == Feature::MotionBlur),
            "{blur_precomp} {blur_group}: {listed:?}"
        );
    }
    let f = variant(|d| {
        let i = layer_at(d, "back");
        d["layers"][i]["masks"] =
            json!([{ "id": "m1", "on": true, "path": { "id": "p", "vertices": [], "p": {} } }]);
    });
    assert!(f.is_empty(), "path masks are drawn: {f:?}");
    // Linear blending and 32-bit precision are drawn; unknown settings are not.
    let f = variant(|d| {
        d["color"] = json!({ "mode": "standard", "linearBlending": true, "precision": "float32" })
    });
    assert!(!f.contains(&Feature::ProjectColor), "{f:?}");
    let f = variant(|d| d["color"] = json!({ "mode": "aces" }));
    assert!(f.contains(&Feature::ProjectColor), "{f:?}");
    // The 3D scene pass draws shadows, every light type, several objects
    // per composition (depth sets), transparency, every primitive and
    // source kind, missing models, and maps; what it routes is listed.
    let scene = |f: &[Feature], what: &str| {
        f.iter()
            .any(|x| matches!(x, Feature::Scene3d(s) if s.contains(what)))
    };
    let no_3d = |f: &[Feature]| !f.iter().any(|x| matches!(x, Feature::Scene3d(_)));
    let f = variant(|d| {
        let i = layer_at(d, "cam");
        d["layers"][i]["d"]["data"]["environment"]["shadows"] = json!(true);
        let i = layer_at(d, "sun");
        d["layers"][i]["d"]["data"]["light"]["castShadow"] = json!(true);
        d["layers"][i]["d"]["data"]["light"]["type"] = json!("point");
    });
    assert!(no_3d(&f), "{f:?}");
    let f = variant(|d| {
        d["layers"].as_array_mut().unwrap().push(object3d(
            "obj2",
            "box",
            json!({}),
            "#FFFFFF",
            W,
            H,
            2.0,
        ));
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["material"]["p"]["opacity"] = ch(json!(0.5));
        d["layers"][i]["d"]["data"]["object"]["source"] = json!({ "primitive": "capsule" });
    });
    assert!(no_3d(&f), "{f:?}");
    // A model whose asset is not in the document is the missing-model box.
    let f = variant(|d| {
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["source"] = json!({ "assetId": "model" });
        d["layers"][i]["d"]["data"]["object"]["material"]["maps"] = json!({ "color": "tex" });
    });
    assert!(no_3d(&f), "{f:?}");
    // An imported model that exists is the glTF lane's.
    let f = variant(|d| {
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["source"] = json!({ "assetId": "model" });
        d["assets"]["model"] =
            json!({ "id": "model", "kind": "model", "format": "glb", "name": "m.glb" });
    });
    assert!(scene(&f, "imported models"), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["material"]["maps"] = json!({ "color": "tex" });
        d["assets"]["tex"] =
            json!({ "id": "tex", "kind": "image", "format": "svg", "name": "t.svg" });
    });
    assert!(scene(&f, "SVG texture maps"), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["generation"] =
            json!({ "groupId": "g", "partId": "p" });
    });
    assert!(scene(&f, "recipe parts"), "{f:?}");
    let f = variant(|d| {
        let i = layer_at(d, "obj");
        d["layers"][i]["d"]["data"]["object"]["source"] = json!({ "primitive": "knot" });
    });
    assert!(scene(&f, "knot primitives"), "{f:?}");
    // Nested compositions are checked too.
    let f = variant(|d| {
        d["comps"]["inner"]["layers"]
            .as_array_mut()
            .unwrap()
            .push(layer("t", "text", json!({}), 0.0, 0.0, 2.0))
    });
    assert!(f.iter().any(|f| matches!(f, Feature::Text(_))), "{f:?}");
}

#[test]
fn requests_are_validated() {
    let (doc, resolver) = everything();
    let doc = load(&doc);
    let mut r = Renderer::headless().unwrap();
    let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    request.region = Some(PixelRect {
        x: 300,
        y: 0,
        width: 40,
        height: 10,
    });
    assert!(
        r.render(&doc, "Ptest-root", ticks(0.0), &request, &resolver)
            .is_err()
    );
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    assert!(
        r.render(&doc, "nope", ticks(0.0), &request, &resolver)
            .is_err()
    );
}

#[test]
fn pass_through_candidates() {
    let fixtures = repo_root().join("apps/desktop/e2e/fixtures");
    let clip = |rotation: f64, extra: bool| {
        let mut v = layer(
            "v",
            "video",
            json!({ "asset": "clip", "fit": "stretch", "w": 160, "h": 90, "trim": 0, "speed": 1 }),
            80.0,
            45.0,
            2.0,
        );
        set(&mut v, "rotation", json!(rotation));
        let mut layers = vec![v];
        if extra {
            layers.insert(
                0,
                layer(
                    "s",
                    "solid",
                    json!({ "color": "#FFFFFF", "w": 10, "h": 10 }),
                    0.0,
                    0.0,
                    2.0,
                ),
            );
        }
        let mut doc = project(160, 90, 2.0, no_fill(), layers);
        asset(&mut doc, "clip", "video", "mp4", 160, 90, 2.0);
        load(&doc)
    };
    let mut resolver = MapResolver::default();
    resolver
        .paths
        .insert("clip".into(), fixtures.join("h264-aac.mp4"));
    let mut r = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float16);
    let p = r
        .pass_through(
            &clip(0.0, false),
            "Ptest-root",
            ticks(0.5),
            &request,
            &resolver,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        p.facts,
        pm_color::chroma::FrameFacts {
            single_source: true,
            untransformed: true,
            effect_free: true
        }
    );
    assert!((p.seconds - 0.5).abs() < 1e-12);
    let p = r
        .pass_through(
            &clip(3.0, false),
            "Ptest-root",
            ticks(0.5),
            &request,
            &resolver,
        )
        .unwrap()
        .unwrap();
    assert!(!p.facts.untransformed, "a rotated frame is resampled");
    assert!(
        r.pass_through(
            &clip(0.0, true),
            "Ptest-root",
            ticks(0.5),
            &request,
            &resolver
        )
        .unwrap()
        .is_none()
    );
    // At another output size the source is resampled.
    let half = RenderRequest::at_scale(Size::new(160, 90), 0.5, Precision::Float16);
    let p = r
        .pass_through(
            &clip(0.0, false),
            "Ptest-root",
            ticks(0.5),
            &half,
            &resolver,
        )
        .unwrap()
        .unwrap();
    assert!(!p.facts.untransformed);
    // An interpreted source is read differently from its file.
    let v = layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": 160, "h": 90, "trim": 0, "speed": 1 }),
        80.0,
        45.0,
        2.0,
    );
    let mut doc = project(160, 90, 2.0, no_fill(), vec![v]);
    asset(&mut doc, "clip", "video", "mp4", 160, 90, 2.0);
    doc["assets"]["clip"]["interpret"] = json!({ "range": "full" });
    assert!(
        r.pass_through(&load(&doc), "Ptest-root", ticks(0.5), &request, &resolver)
            .unwrap()
            .is_none()
    );
}

#[test]
fn render_into_a_callers_target_equals_render() {
    let (doc, resolver) = everything();
    let doc = load(&doc);
    let mut r = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    let (expected, _, _) = render_bytes(&mut r, &doc, 0.4, &request, &resolver);
    // A presenter's working texture (or an export buffer).
    let target = pm_gpu::Target::new(
        r.device(),
        "caller",
        W,
        H,
        wgpu::TextureFormat::Rgba16Float,
        pm_gpu::target::working_usage(),
    );
    let out = r
        .render_into(
            &doc,
            "Ptest-root",
            ticks(0.4),
            &request,
            &resolver,
            &target,
            None,
        )
        .unwrap();
    assert!(out.failures.is_empty());
    let got = pm_gpu::readback::read_raw(r.device(), r.queue(), &target.texture).unwrap();
    assert!(got == expected);
    // The bridge presenter's working texture (render target, sampled,
    // copied out; no storage or copy-in usage).
    let presenter = pm_gpu::bridge_out::display::WorkingTexture::new(r.device(), Size::new(W, H));
    let wrapped = pm_gpu::Target::wrap(presenter.texture.clone());
    r.render_into(
        &doc,
        "Ptest-root",
        ticks(0.4),
        &request,
        &resolver,
        &wrapped,
        None,
    )
    .unwrap();
    let got = pm_gpu::readback::read_raw(r.device(), r.queue(), &presenter.texture).unwrap();
    assert!(
        got == expected,
        "the presenter's texture holds the same frame"
    );
    // A target of the wrong size is refused.
    let small = pm_gpu::Target::new(
        r.device(),
        "small",
        10,
        10,
        wgpu::TextureFormat::Rgba16Float,
        pm_gpu::target::working_usage(),
    );
    assert!(
        r.render_into(
            &doc,
            "Ptest-root",
            ticks(0.4),
            &request,
            &resolver,
            &small,
            None
        )
        .is_err()
    );
}

#[test]
fn contract_2_requests_convert() {
    use pm_protocol::color::OutputColor;
    use pm_protocol::common::{CompositionId, Ticks};
    use pm_protocol::frame::{
        FramePurpose, FrameRequest, MediaPreference, MotionBlurKind, PreviewResolution,
    };
    use pm_protocol::priority::PriorityClass;
    let mut request = FrameRequest {
        composition: CompositionId::new("Ptest-root"),
        time: Ticks::ZERO,
        purpose: FramePurpose::Export,
        priority: PriorityClass::UserJob,
        output_size: Size::new(W, H),
        region: Some(PixelRect {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        }),
        scale: 1.0,
        resolution: PreviewResolution::Full,
        media: MediaPreference::Originals,
        color: OutputColor::SrgbImage,
        precision: Precision::Float32,
        motion_blur: MotionBlurKind::Export,
        view: None,
        draft_3d: false,
        draft_render: false,
        depth_id: true,
        overlay: false,
    };
    // Final outputs carry no depth and id surface (contract 2).
    assert!(RenderRequest::from_frame_request(&request).is_err());
    request.depth_id = false;
    let r = RenderRequest::from_frame_request(&request).unwrap();
    assert!(r.export && !r.depth_id && r.precision == Precision::Float32);
    let mut viewer = request.clone();
    viewer.purpose = FramePurpose::Viewer;
    viewer.priority = PriorityClass::Interactive;
    viewer.depth_id = true;
    viewer.color = OutputColor::Viewer {
        space: pm_protocol::color::ViewerColorSpace::Srgb,
    };
    let v = RenderRequest::from_frame_request(&viewer).unwrap();
    assert!(!v.export && v.depth_id);
    assert_eq!(
        (r.output_size, r.region, r.scale),
        (Size::new(W, H), request.region, Some((1.0, 1.0)))
    );
    assert_eq!(r.target_size(), Size::new(100, 50));
    // The frame renders at the region's size.
    let (doc, resolver) = everything();
    let mut renderer = Renderer::headless().unwrap();
    let frame = renderer
        .render(&load(&doc), "Ptest-root", ticks(0.0), &r, &resolver)
        .unwrap();
    assert_eq!((frame.color.width(), frame.color.height()), (100, 50));
    // An invalid request is refused.
    request.scale = 40.0;
    assert!(RenderRequest::from_frame_request(&request).is_err());
}

/// Frame time of the composition above at 1920 × 1080 (run with
/// `--ignored --release`; reported, not gated: Phase 2's performance rows
/// are measured in the app).
#[test]
#[ignore]
fn frame_time_at_1080p() {
    let (doc, resolver) = everything();
    let doc = load(&doc);
    let mut r = Renderer::headless().unwrap();
    for precision in [Precision::Float16, Precision::Float32] {
        let request = RenderRequest::at_scale(Size::new(W, H), 6.0, precision);
        let mut times = Vec::new();
        for i in 0..40 {
            let t0 = std::time::Instant::now();
            let frame = r
                .render(
                    &doc,
                    "Ptest-root",
                    ticks(0.5 + f64::from(i) / 30.0),
                    &request,
                    &resolver,
                )
                .unwrap();
            let _ = r.device().poll(wgpu::PollType::wait_indefinitely());
            if i >= 5 {
                times.push(t0.elapsed().as_secs_f64() * 1000.0);
            }
            drop(frame);
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "{precision:?} 1920×1080: median {:.2} ms, p95 {:.2} ms (CPU plan + GPU, video decode included)",
            times[times.len() / 2],
            times[times.len() * 95 / 100]
        );
    }
}

/// A 24-bit uncompressed TGA of `w` × `h` (a container the renderer does
/// not draw: it decodes, through ImageIO, as an unrecognized format).
fn tga(w: u16, h: u16) -> Vec<u8> {
    let mut b = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&h.to_le_bytes());
    b.extend_from_slice(&[24, 0x20]);
    b.extend(std::iter::repeat_n(0x80u8, w as usize * h as usize * 3));
    b
}

#[test]
fn exec_time_unsupported_media_leaves_the_target_untouched() {
    let (doc, resolver) = everything();
    let doc = load(&doc);
    let mut r = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    let target = pm_gpu::Target::new(
        r.device(),
        "caller",
        W,
        H,
        wgpu::TextureFormat::Rgba16Float,
        pm_gpu::target::working_usage(),
    );
    let rendered = r
        .render_into(
            &doc,
            "Ptest-root",
            ticks(0.4),
            &request,
            &resolver,
            &target,
            None,
        )
        .unwrap();
    // The good frame draws every layer, the still included (the e2e
    // fixtures must be present), so the comparison below is not vacuous.
    assert!(
        rendered.failures.is_empty(),
        "the first frame draws: {:?}",
        rendered.failures
    );
    let before = pm_gpu::readback::read_raw(r.device(), r.queue(), &target.texture).unwrap();
    // The document records a PNG; the file is a TGA: only the media probe
    // at exec time can tell.
    let dir = std::env::temp_dir().join(format!("pm-render-exec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let still = dir.join("still.png");
    std::fs::write(&still, tga(160, 90)).unwrap();
    let mut bad = MapResolver {
        paths: resolver.paths.clone(),
    };
    bad.paths.insert("still".into(), still);
    let err = r
        .render_into(
            &doc,
            "Ptest-root",
            ticks(0.4),
            &request,
            &bad,
            &target,
            None,
        )
        .err()
        .expect("an unsupported frame");
    assert!(
        matches!(&err, pm_render::RenderError::Unsupported(u)
            if u.iter().any(|u| u.feature == Feature::Media("stills of an unrecognized format".into()))),
        "{err:?}"
    );
    let after = pm_gpu::readback::read_raw(r.device(), r.queue(), &target.texture).unwrap();
    assert!(after == before, "the caller's target keeps its frame");
    // The renderer is still usable, and its caches are consistent.
    r.render_into(
        &doc,
        "Ptest-root",
        ticks(0.4),
        &request,
        &resolver,
        &target,
        None,
    )
    .unwrap();
    let again = pm_gpu::readback::read_raw(r.device(), r.queue(), &target.texture).unwrap();
    assert!(again == before);
    std::fs::remove_dir_all(&dir).ok();
}

/// A two-second 160 × 90 H.264 clip of one color, tagged BT.709.
fn solid_clip(path: &std::path::Path, color: &str) {
    let bin = std::env::var_os("PM_FFMPEG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
        .join("bin/ffmpeg");
    let source = format!("color=c={color}:size=160x90:rate=30:duration=2");
    let ok = std::process::Command::new(bin)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", &source])
        .args([
            "-c:v",
            "h264_videotoolbox",
            "-b:v",
            "2M",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(["-color_primaries", "bt709", "-color_trc", "bt709"])
        .args(["-colorspace", "bt709"])
        .arg(path)
        .status()
        .expect("ffmpeg (PM_FFMPEG_DIR)")
        .success();
    assert!(ok);
}

#[test]
fn a_video_replaced_on_disk_is_reopened() {
    let v = layer(
        "v",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": 160, "h": 90, "trim": 0, "speed": 1 }),
        0.0,
        0.0,
        2.0,
    );
    let mut doc = project(160, 90, 2.0, no_fill(), vec![v]);
    asset(&mut doc, "clip", "video", "mp4", 160, 90, 2.0);
    let doc = load(&doc);
    let dir = std::env::temp_dir().join(format!("pm-render-replace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clip.mp4");
    let (blue, yellow) = (dir.join("blue.mp4"), dir.join("yellow.mp4"));
    solid_clip(&blue, "0x2040C0");
    solid_clip(&yellow, "0xE0C020");
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.clone());
    let request = RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float16);
    let frame = |r: &mut Renderer| render_bytes(r, &doc, 0.4, &request, &resolver).0;
    std::fs::copy(&blue, &path).unwrap();
    let mut r = Renderer::headless().unwrap();
    let first = frame(&mut r);
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::copy(&yellow, &path).unwrap();
    let replaced = frame(&mut r);
    let fresh = frame(&mut Renderer::headless().unwrap());
    assert!(
        replaced != first,
        "the replaced file is read, not the stale index"
    );
    assert!(replaced == fresh);
    std::fs::remove_dir_all(&dir).ok();
}

/// The overlay geometry a render reports (section 18 item 6): each
/// top-level layer's quad in output pixels, top-down, where its pixels are.
#[test]
fn layer_quads_cover_the_drawn_pixels_top_down() {
    let solid = layer(
        "box",
        "solid",
        json!({ "color": "#FF0000", "w": 40, "h": 20 }),
        70.0,
        30.0,
        2.0,
    );
    let mut rotated = layer(
        "tilt",
        "solid",
        json!({ "color": "#00FF00", "w": 30, "h": 30 }),
        240.0,
        130.0,
        2.0,
    );
    set(&mut rotated, "rotation", json!(30));
    let doc = load(&project(W, H, 2.0, no_fill(), vec![solid, rotated]));
    let mut r = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(W, H), 2.0, Precision::Float16);
    let size = request.target_size();
    let target = pm_gpu::Target::new(
        r.device(),
        "quads",
        size.width,
        size.height,
        wgpu::TextureFormat::Rgba16Float,
        pm_gpu::target::working_usage(),
    );
    let out = r
        .render_into(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
            &target,
            None,
        )
        .unwrap();
    let frame = r
        .render(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request,
            &MapResolver::default(),
        )
        .unwrap();
    let rgba = r.read_rgba(&frame).unwrap();
    assert_eq!(out.quads.len(), 2, "{:?}", out.quads);
    for (quad, channel) in out.quads.iter().map(|q| {
        let channel = if q.layer == "box" { 0 } else { 1 };
        (q, channel)
    }) {
        // Pixels where the layer's color is fully drawn.
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for y in 0..size.height {
            for x in 0..size.width {
                let i = ((y * size.width + x) * 4) as usize;
                if rgba[i + channel] > 0.99 && rgba[i + 3] > 0.99 {
                    x0 = x0.min(f64::from(x));
                    y0 = y0.min(f64::from(y));
                    x1 = x1.max(f64::from(x) + 1.0);
                    y1 = y1.max(f64::from(y) + 1.0);
                }
            }
        }
        let qx0 = quad.corners.iter().map(|c| c[0]).fold(f64::MAX, f64::min);
        let qx1 = quad.corners.iter().map(|c| c[0]).fold(f64::MIN, f64::max);
        let qy0 = quad.corners.iter().map(|c| c[1]).fold(f64::MAX, f64::min);
        let qy1 = quad.corners.iter().map(|c| c[1]).fold(f64::MIN, f64::max);
        // Fully covered pixels lie inside the quad's box, within the
        // antialiased edge (1.5 output pixels).
        assert!(
            qx0 - 1.5 <= x0 && x1 <= qx1 + 1.5 && qy0 - 1.5 <= y0 && y1 <= qy1 + 1.5,
            "{}: pixels {x0},{y0}..{x1},{y1}, quad {qx0},{qy0}..{qx1},{qy1}",
            quad.layer
        );
        assert!(
            (x0 - qx0).abs() < 3.0 && (y0 - qy0).abs() < 3.0 || quad.layer == "tilt",
            "{}: pixels start at {x0},{y0}, the quad at {qx0},{qy0}",
            quad.layer
        );
    }
}
