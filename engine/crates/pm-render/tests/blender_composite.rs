//! Blender surfaces in the render graph (plan sections 8, 9, 17; section
//! 25's "Blender surfaces" class: the engine composite of the stored
//! surfaces, exact): each surface decodes as `code / 65535`, premultiplies
//! in E at f32, and composites per layer as a source node over its crop
//! rectangle, in the plan's layer order, with the layer's opacity. A final
//! render without a layer's surface fails that layer with today's sentence;
//! a composition without `render3d.enabled` keeps native shading; the
//! paused preview draws what it has and asks for the rest; nested
//! compositions are found at their own frame.

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pm_blender::snapshot::Snapshot;
use pm_blender::surface::encode_rgba16;
use pm_blender::{DecodedSurface, decode_surface};
use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::blender::{BlenderRender, NOT_PREPARED, PreviewSurfaces, SnapshotOptions};
use pm_render::plan::{CompPlan, Content, Source};
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const W: u32 = 96;
const H: u32 = 64;

fn scene(render3d: bool) -> Value {
    let mut a = object3d("a", "box", json!({}), "#FF8040", W, H, 2.0);
    let mut b = object3d("b", "sphere", json!({}), "#4080FF", W, H, 2.0);
    set(&mut a, "position.x", json!(-0.6));
    set(&mut b, "position.x", json!(0.6));
    set(&mut b, "position.z", json!(0.5));
    set(&mut b, "opacity", json!(50));
    let mut doc = project(
        W,
        H,
        2.0,
        solid_fill("#102030"),
        vec![
            camera("cam", [0.0, 0.0, 4.0], false, 2.0),
            sun("sun", [-3.0, 5.0, 4.0], false, 2.0),
            a,
            b,
        ],
    );
    if render3d {
        doc["render3d"] = json!({ "enabled": true, "engine": "eevee", "samples": 16 });
    }
    doc
}

/// A deterministic 16-bit straight-alpha surface with content in
/// `[x0, x1) × [y0, y1)` (alpha ramps; colors cover the full code range).
fn surface(w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32, seed: u32) -> Vec<u8> {
    let mut rgba = vec![0u16; (w * h * 4) as usize];
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * w + x) * 4) as usize;
            let k = x * 7 + y * 13 + seed * 101;
            rgba[i] = ((k * 977) % 65536) as u16;
            rgba[i + 1] = ((k * 3011 + 17) % 65536) as u16;
            rgba[i + 2] = ((k * 5003 + 400) % 65536) as u16;
            rgba[i + 3] = if (x + y) % 5 == 0 {
                65535
            } else {
                ((k * 1499) % 65536) as u16
            };
        }
    }
    // ImageBitmap's straight upload keeps this fringe even at zero alpha.
    if x0 > 0 {
        let fringe = ((y0 * w + x0 - 1) * 4) as usize;
        rgba[fringe..fringe + 4].copy_from_slice(&[40000, 50000, 60000, 0]);
    }
    // Color chunks the engine must ignore.
    encode_rgba16(
        w,
        h,
        &rgba,
        &[(b"gAMA", &45455u32.to_be_bytes()), (b"sRGB", &[0])],
    )
}

fn request(precision: Precision) -> RenderRequest {
    RenderRequest::sized(Size::new(W, H), precision)
}

/// The bottom-to-top layer ids of the root plan's draws.
fn draw_order(plan: &CompPlan) -> Vec<String> {
    plan.draws
        .iter()
        .filter(|d| {
            matches!(
                &d.content,
                Content::Texture {
                    source: Source::Blender(_),
                    ..
                }
            )
        })
        .map(|d| d.layer.clone())
        .collect()
}

/// f64 reference: the background, then each surface (premultiplied E,
/// times the layer's opacity) source-over.
fn reference(
    order: &[String],
    surfaces: &HashMap<String, Arc<DecodedSurface>>,
    opacity: &HashMap<&str, f64>,
    compatibility: bool,
    precision: Precision,
) -> Vec<f64> {
    let bg = [16.0 / 255.0, 32.0 / 255.0, 48.0 / 255.0, 1.0];
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            // Metal truncates f16 target writes after each blend. Model
            // that storage in the straight-upload reference too; the
            // original level-2 reference and tolerances stay unchanged.
            let round = |v: f64| {
                if !compatibility || precision != Precision::Float16 {
                    return v;
                }
                let mut bits = pm_gpu::readback::f32_to_half(v as f32);
                if f64::from(pm_gpu::readback::half_to_f32(bits)).abs() > v.abs() {
                    bits -= 1;
                }
                f64::from(pm_gpu::readback::half_to_f32(bits))
            };
            let mut c = bg.map(round);
            for id in order {
                let surface = &surfaces[id];
                let p = if compatibility
                    && x >= surface.crop.x
                    && y >= surface.crop.y
                    && x < surface.crop.x + surface.crop.width
                    && y < surface.crop.y + surface.crop.height
                {
                    let i = (((y - surface.crop.y) * surface.crop.width + x - surface.crop.x) * 4)
                        as usize;
                    std::array::from_fn(|k| {
                        (surface.straight[i + k] * 255.0).round_ties_even() / 255.0
                    })
                } else {
                    surface.pixel(x, y)
                };
                let o = opacity[id.as_str()];
                let s = [
                    f64::from(p[0]) * o,
                    f64::from(p[1]) * o,
                    f64::from(p[2]) * o,
                    f64::from(p[3]) * o,
                ];
                for k in 0..4 {
                    c[k] = round(s[k] + c[k] * (1.0 - s[3]));
                }
            }
            out.extend(c);
        }
    }
    out
}

fn max_diff(a: &[f32], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (f64::from(*x) - y).abs())
        .fold(0.0, f64::max)
}

#[test]
fn surfaces_composite_exactly_per_layer_at_level2() {
    // Blender is retired in the app; these tests run its stage.
    pm_render::blender::enable_for_tests();
    check_surfaces(2);
}

#[test]
fn level1_matches_the_straight_imagebitmap_upload_including_zero_alpha_rgb() {
    // Blender is retired in the app; these tests run its stage.
    pm_render::blender::enable_for_tests();
    check_surfaces(1);
}

fn check_surfaces(compat: u8) {
    let mut exact = scene(true);
    for layer in exact["layers"].as_array_mut().unwrap() {
        if compat >= 2 {
            layer["compat"] = json!(compat);
        }
    }
    let doc = load(&exact);
    let mut r = Renderer::headless().expect("a Metal device");
    let assets = MapResolver::default();
    // The frame's Blender snapshots: one frame, both layers.
    let snaps = r
        .blender_snapshots(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &request(Precision::Float32),
            &assets,
            false,
            SnapshotOptions::default(),
        )
        .unwrap();
    assert!(snaps.errors.is_empty(), "{:?}", snaps.errors);
    assert_eq!(snaps.snapshots.len(), 1);
    let snap = &snaps.snapshots[0];
    assert_eq!(snap.layers(), ["a", "b"]);
    let legacy = snap
        .json
        .get("compatibilityLayers")
        .and_then(pm_model::Value::as_array);
    if compat < 2 {
        assert_eq!(legacy.unwrap().len(), 2);
    } else {
        assert!(legacy.is_none());
    }
    assert_eq!(snap.size(), (W, H));
    assert_eq!(snap.frame.composition_path, ["Ptest-root"]);
    let decoded: HashMap<String, Arc<DecodedSurface>> = [
        ("a".to_string(), surface(W, H, 5, 7, 60, 50, 1)),
        ("b".to_string(), surface(W, H, 30, 3, 91, 61, 2)),
    ]
    .into_iter()
    .map(|(k, png)| (k, Arc::new(decode_surface(&png).unwrap())))
    .collect();
    assert_eq!(decoded["a"].crop.x, 3);
    let mut surfaces = pm_blender::job::Surfaces::new();
    surfaces.insert(snap.frame.lookup(), decoded.clone());
    let opacity: HashMap<&str, f64> = [("a", 1.0), ("b", 0.5)].into();
    for precision in [Precision::Float32, Precision::Float16] {
        let mut req = request(precision);
        req.blender = BlenderRender::Final(Arc::new(surfaces.clone()));
        let (plan, failures) = r
            .plan(&doc, "Ptest-root", ticks(0.0), &req, &assets)
            .unwrap();
        assert!(failures.is_empty(), "{failures:?}");
        let order = draw_order(&plan);
        assert_eq!(order.len(), 2, "both layers draw their surfaces");
        let frame = r
            .render(&doc, "Ptest-root", ticks(0.0), &req, &assets)
            .unwrap();
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        let got = r.read_rgba(&frame).unwrap();
        let want = reference(&order, &decoded, &opacity, compat < 2, precision);
        let d = max_diff(&got, &want);
        eprintln!("{precision:?}: max |engine − f64 reference| = {d:e}");
        match precision {
            // f32 storage and blending: rounding only.
            Precision::Float32 => assert!(d <= 2e-7, "{d}"),
            // Half-float surfaces and accumulation.
            _ => assert!(d <= 1.5e-3, "{d}"),
        }
    }
}

#[test]
fn a_missing_surface_fails_the_layer_and_native_shading_needs_no_blender() {
    // Blender is retired in the app; these tests run its stage.
    pm_render::blender::enable_for_tests();
    let doc = load(&scene(true));
    let mut r = Renderer::headless().expect("a Metal device");
    let assets = MapResolver::default();
    let mut req = request(Precision::Float16);
    req.blender = BlenderRender::Final(Arc::new(Default::default()));
    let frame = r
        .render(&doc, "Ptest-root", ticks(0.0), &req, &assets)
        .unwrap();
    let mut messages: Vec<(String, String)> = frame
        .failures
        .iter()
        .map(|f| (f.layer.clone(), f.message.clone()))
        .collect();
    messages.sort();
    assert_eq!(
        messages,
        [
            ("a".to_string(), NOT_PREPARED.to_string()),
            ("b".to_string(), NOT_PREPARED.to_string())
        ]
    );
    // Without `render3d.enabled` the native pass draws and nothing fails.
    let native = load(&scene(false));
    let frame = r
        .render(&native, "Ptest-root", ticks(0.0), &req, &assets)
        .unwrap();
    assert!(frame.failures.is_empty());
    let snaps = r
        .blender_snapshots(
            &native,
            "Ptest-root",
            ticks(0.0),
            &req,
            &assets,
            false,
            SnapshotOptions::default(),
        )
        .unwrap();
    assert!(snaps.snapshots.is_empty());
}

struct Preview {
    have: HashMap<(Vec<u8>, String), Arc<DecodedSurface>>,
    wanted: Mutex<Vec<Snapshot>>,
}

impl PreviewSurfaces for Preview {
    fn options(&self) -> SnapshotOptions {
        SnapshotOptions {
            force_eevee: true,
            ..Default::default()
        }
    }
    fn lookup(&self, snapshot: &Snapshot, layer: &str) -> Option<Arc<DecodedSurface>> {
        self.have
            .get(&(snapshot.hash().0.to_vec(), layer.to_owned()))
            .cloned()
    }
    fn wanted(&self, snapshot: Snapshot) {
        self.wanted.lock().unwrap().push(snapshot);
    }
}

#[test]
fn the_paused_preview_draws_what_it_has_and_asks_for_the_rest() {
    // Blender is retired in the app; these tests run its stage.
    pm_render::blender::enable_for_tests();
    let doc = load(&scene(false));
    let mut r = Renderer::headless().expect("a Metal device");
    let assets = MapResolver::default();
    let req = request(Precision::Float16);
    // Preview snapshots are made at the preview scale with EEVEE, even
    // without `render3d.enabled` (today's rendered preview).
    let snaps = r
        .blender_snapshots(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &req,
            &assets,
            true,
            SnapshotOptions {
                force_eevee: true,
                ..Default::default()
            },
        )
        .unwrap();
    let snap = snaps.snapshots[0].clone();
    assert_eq!(snap.size(), (W / 2, H / 2));
    assert!(snap.frame.preview);
    // The preview frame is the composition's own size whatever the
    // viewer's zoom (today's `comp.w × previewScale`): a zoomed viewer
    // asks for the same snapshot.
    let zoomed = r
        .blender_snapshots(
            &doc,
            "Ptest-root",
            ticks(0.0),
            &RenderRequest::sized(Size::new(W * 3, H * 3), Precision::Float16),
            &assets,
            true,
            SnapshotOptions {
                force_eevee: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(zoomed.snapshots[0].hash(), snap.hash());
    let s = snap.settings().unwrap();
    assert_eq!(s.get("samples").and_then(|v| v.as_f64()), Some(16.0));
    let half = Arc::new(decode_surface(&surface(W / 2, H / 2, 2, 2, 40, 30, 3)).unwrap());
    let preview = Arc::new(Preview {
        have: [((snap.hash().0.to_vec(), "a".to_string()), half.clone())].into(),
        wanted: Mutex::new(vec![]),
    });
    let mut req = request(Precision::Float16);
    req.blender = BlenderRender::Preview(preview.clone());
    let (plan, failures) = r
        .plan(&doc, "Ptest-root", ticks(0.0), &req, &assets)
        .unwrap();
    assert!(failures.is_empty());
    // `a` draws its half-size surface stretched over the frame; `b` draws
    // natively while its surface is asked for.
    let blender: Vec<(&str, [f64; 6])> = plan
        .draws
        .iter()
        .filter(|d| {
            matches!(
                &d.content,
                Content::Texture {
                    source: Source::Blender(_),
                    ..
                }
            )
        })
        .map(|d| (d.layer.as_str(), d.matrix))
        .collect();
    assert_eq!(blender.len(), 1);
    assert_eq!(blender[0].0, "a");
    let rect = half.rect();
    assert_eq!(
        blender[0].1,
        [
            rect[2] * f64::from(W),
            0.0,
            0.0,
            rect[3] * f64::from(H),
            rect[0] * f64::from(W),
            rect[1] * f64::from(H)
        ]
    );
    assert!(plan.draws.iter().any(|d| d.layer == "b"
        && matches!(
            &d.content,
            Content::Texture {
                source: Source::Object3d(_),
                ..
            }
        )));
    let wanted = preview.wanted.lock().unwrap();
    assert_eq!(wanted.len(), 1);
    assert_eq!(wanted[0].hash(), snap.hash());
}

#[test]
fn nested_compositions_are_their_own_frames() {
    // Blender is retired in the app; these tests run its stage.
    pm_render::blender::enable_for_tests();
    let mut doc = project(W, H, 2.0, solid_fill("#000000"), vec![]);
    let mut inner = scene(true);
    let layers = inner["layers"].take().as_array().unwrap().clone();
    composition(&mut doc, "inner", 48, 32, 2.0, layers);
    doc["comps"]["inner"]["render3d"] = json!({ "enabled": true });
    let mut pre = layer(
        "pre",
        "precomp",
        json!({ "comp": "inner", "w": 48, "h": 32 }),
        48.0,
        32.0,
        2.0,
    );
    set(&mut pre, "scale.x", json!(200));
    set(&mut pre, "scale.y", json!(200));
    doc["layers"] = json!([pre]);
    let doc = load(&doc);
    let mut r = Renderer::headless().expect("a Metal device");
    let assets = MapResolver::default();
    let req = request(Precision::Float32);
    let snaps = r
        .blender_snapshots(
            &doc,
            "Ptest-root",
            ticks(0.5),
            &req,
            &assets,
            false,
            SnapshotOptions::default(),
        )
        .unwrap();
    assert!(snaps.errors.is_empty(), "{:?}", snaps.errors);
    assert_eq!(snaps.snapshots.len(), 1);
    let snap = &snaps.snapshots[0];
    assert_eq!(snap.frame.composition_path, ["Ptest-root", "inner"]);
    // The precomp renders at its own size times the output scale (1 here).
    assert_eq!((snap.frame.width, snap.frame.height), (48, 32));
    let mut frame = HashMap::new();
    for l in snap.layers() {
        frame.insert(
            l,
            Arc::new(decode_surface(&surface(48, 32, 0, 0, 48, 32, 4)).unwrap()),
        );
    }
    let mut surfaces = pm_blender::job::Surfaces::new();
    surfaces.insert(snap.frame.lookup(), frame);
    let mut req = request(Precision::Float32);
    req.blender = BlenderRender::Final(Arc::new(surfaces));
    let frame = r
        .render(&doc, "Ptest-root", ticks(0.5), &req, &assets)
        .unwrap();
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
}
