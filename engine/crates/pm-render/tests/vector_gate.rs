//! G-V1 (docs/engine/gate-policy.md): shape and editable-path layers
//! against an exact reference, with today's Canvas2D rasters as the
//! witness.
//!
//! Each case of `fixtures/shape-cases.json` (every shape kind and property,
//! editable-path scenes, at raster scales 0.25 to 4) is today's
//! `rasterShape`/`rasterPaths` canvas: its size and the transform onto it
//! are today's. The engine covers the same geometry on that canvas with its
//! production path (`VectorCache`: exact area coverage, float paint) in
//! f32. The reference covers it exactly (16 × 16 cells per
//! pixel; strokes as the region their joins and caps define,
//! `pm_reference::stroke`). Today is the capture of those canvases in the
//! app (`tools/canvas-ref` of the retired emulation, kept outside the
//! repository as `canvas-*`); without one the gate measures the engine
//! against the reference alone and says so.
//!
//! Rules: interiors within 1 CV of today (the authored colours); edges at
//! least as close to the reference as today at max, p99.9 and mean, and per
//! pixel within today's error + 1 CV for 99.9% and + 4 CV for any pixel;
//! the engine's alpha area and centroid equal the reference's within 1e-3.

mod support;

use pm_protocol::color::Precision;
use pm_reference::compare::{Bounds, Class, Stats, classify, compare, geometry, same_geometry};
use pm_render::Renderer;
use pm_render::vector::VectorSource;
use pm_render::vector::cache::VectorCache;
use pm_vector::kurbo::Affine;
use pm_vector::path::{PathColors, PathLayer, PathNode, PathValues, Vertex};
use pm_vector::render::scene::{CoverageScene, ItemStyle, PixelRect};
use pm_vector::shape::{ShapeColors, ShapeContent, ShapeKind, ShapeRaster, shape_geometry};
use serde_json::Value;

fn cases() -> Vec<Value> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/shape-cases.json")).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    v["cases"].as_array().unwrap().clone()
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap(),
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => f64::NAN,
    }
}

fn field(d: &Value, k: &str) -> f64 {
    d.get(k).map_or(f64::NAN, num)
}

fn s(v: &Value) -> String {
    v.as_str().map(str::to_owned).unwrap_or_default()
}

/// A case's canvas and scene: today's raster geometry, the engine's items.
/// `None` when the content draws nothing (today's raster throws).
fn scene(c: &Value) -> Option<((u32, u32), CoverageScene)> {
    let scale = num(&c["scale"]);
    let mut scene = CoverageScene::default();
    if c["kind"] == "shape" {
        let d = &c["d"];
        let content = ShapeContent {
            shape: ShapeKind::from_name(d["shape"].as_str().unwrap_or("")),
            w: num(&d["w"]),
            h: num(&d["h"]),
            radius: field(d, "radius"),
            independent_corners: d["independentCorners"].as_bool().unwrap_or(false),
            corner_radii: ["radiusTL", "radiusTR", "radiusBR", "radiusBL"]
                .map(|k| d.get(k).filter(|v| !v.is_null()).map(num)),
            smoothing: field(d, "smoothing"),
            stroke: field(d, "stroke"),
            points: field(d, "points"),
        };
        let colors = ShapeColors {
            color: s(&d["color"]),
            stroke_color: s(&d["strokeColor"]),
        };
        let g = ShapeRaster::new(&content, scale);
        let geometry = shape_geometry(&content).ok()?;
        scene.push_shape(&geometry, g.shape_to_pixels(), &colors.color, &colors.stroke_color);
        return Some(((g.width as u32, g.height as u32), scene));
    }
    let mut nodes = vec![];
    let mut colors = vec![];
    for p in c["paths"].as_array().unwrap() {
        let v = &p["p"];
        let n = |k: &str| num(&v[k]);
        nodes.push(PathNode {
            id: s(&p["id"]),
            parent: p["parent"].as_str().map(str::to_owned),
            values: PathValues {
                x: n("x"),
                y: n("y"),
                rotation: n("rotation"),
                scale_x: n("scaleX"),
                scale_y: n("scaleY"),
                closed: v["closed"].as_bool().unwrap_or(false),
                fill_enabled: v["fillEnabled"].as_bool().unwrap_or(false),
                fill_opacity: n("fillOpacity"),
                stroke_width: n("strokeWidth"),
                stroke_opacity: n("strokeOpacity"),
                trim_start: n("trimStart"),
                trim_end: n("trimEnd"),
                trim_offset: n("trimOffset"),
                copies: n("copies"),
                repeat_x: n("repeatX"),
                repeat_y: n("repeatY"),
                repeat_rotation: n("repeatRotation"),
                vertices: p["vertices"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|q| Vertex {
                        x: num(&q["x"]),
                        y: num(&q["y"]),
                        in_x: q.get("inX").map_or(0.0, num),
                        in_y: q.get("inY").map_or(0.0, num),
                        out_x: q.get("outX").map_or(0.0, num),
                        out_y: q.get("outY").map_or(0.0, num),
                    })
                    .collect(),
            },
        });
        colors.push(PathColors {
            fill: s(&v["fill"]),
            stroke: s(&v["stroke"]),
        });
    }
    let layer = PathLayer::new(&nodes);
    let raster = layer.raster(scale);
    let to_pixels: Affine = layer.layer_to_pixels(&raster);
    let fills: Vec<String> = colors.iter().map(|c| c.fill.clone()).collect();
    let strokes: Vec<String> = colors.iter().map(|c| c.stroke.clone()).collect();
    scene.push_paths(&layer, to_pixels, &fills, &strokes);
    Some(((raster.width.max(1.0) as u32, raster.height.max(1.0) as u32), scene))
}

/// The engine's pixels of `scene` on the canvas, premultiplied, f32.
fn engine(r: &Renderer, cache: &mut VectorCache, scene: &CoverageScene, (w, h): (u32, u32)) -> Vec<f64> {
    let rect = PixelRect::new(0, 0, w, h);
    if scene.pixel_bounds(rect).is_none() {
        return vec![0.0; (w * h * 4) as usize];
    }
    let source = VectorSource::Scene {
        scene: scene.clone(),
        rect,
    };
    let view = cache.resolve(r.device(), r.queue(), &source, Precision::Float32).unwrap();
    pm_gpu::readback::read_f32(r.device(), r.queue(), view.texture())
        .unwrap()
        .into_iter()
        .map(f64::from)
        .collect()
}

/// Pooled per-pixel errors (largest channel, CV) of one pixel class.
#[derive(Default)]
struct Pool {
    engine: Vec<f64>,
    today: Vec<f64>,
}

fn cv(a: &[f64], b: &[f64]) -> f64 {
    (0..4).map(|c| (a[c] - b[c]).abs() * 255.0).fold(0.0, f64::max)
}

#[test]
fn shapes_and_paths_meet_the_reference() {
    let capture = support::newest_capture("canvas-");
    match &capture {
        Some(dir) => eprintln!("today: {}", dir.display()),
        None => eprintln!("note: no canvas-* capture; the engine is judged against the reference alone"),
    }
    let r = Renderer::headless().unwrap();
    let mut failures = Vec::new();
    let (mut compared, mut stroked_pool, mut all_pool) = (0, Pool::default(), Pool::default());
    let mut cache = VectorCache::new();
    {
        for c in cases() {
            let id = c["id"].as_str().unwrap();
            if std::env::var("PM_GATE_ONLY").is_ok_and(|o| !id.starts_with(&o)) {
                continue;
            }
            let Some(((w, h), scene)) = scene(&c) else {
                continue;
            };
            let (wu, hu) = (w as usize, h as usize);
            let ours = engine(&r, &mut cache, &scene, (w, h));
            let reference = support::reference::scene_pixels(&scene, wu, hu).unwrap();
            let edge: Vec<bool> = reference.iter().map(|p| p.1).collect();
            let reference: Vec<f64> = reference.iter().flat_map(|p| p.0).collect();
            let classes = classify(&edge, &vec![false; wu * hu], wu, hu, 2);
            let ref_alpha: Vec<f64> = reference.chunks(4).map(|p| p[3]).collect();
            let our_alpha: Vec<f64> = ours.chunks(4).map(|p| p[3]).collect();
            if ref_alpha.iter().sum::<f64>() > 0.0
                && let Err(e) = same_geometry(&geometry(&our_alpha, wu), &geometry(&ref_alpha, wu), 1e-3)
            {
                failures.push(format!("{id}: geometry {e}"));
            }
            let today = capture.as_ref().map(|dir| {
                let (cw, ch, px) = support::read_png(&dir.join(format!("{id}.png")));
                assert_eq!((cw, ch), (w, h), "{id}: today's canvas size");
                px.iter().map(|v| f64::from(*v) / 255.0).collect::<Vec<f64>>()
            });
            if let Some(today) = &today {
                compared += 1;
                let report = compare(&ours, today, &reference, &classes);
                let failed = report.failures(&Bounds::EIGHT_BIT);
                for f in &failed {
                    failures.push(format!("{id}: {f}"));
                }
                // Side by side (engine, today, reference), outside the
                // repository: PM_GATE_EVIDENCE=dir, failing cases only unless
                // PM_GATE_EVIDENCE_ALL is set.
                if let Ok(dir) = std::env::var("PM_GATE_EVIDENCE")
                    && (!failed.is_empty() || std::env::var_os("PM_GATE_EVIDENCE_ALL").is_some())
                {
                    let dir = std::path::Path::new(&dir);
                    std::fs::create_dir_all(dir).unwrap();
                    let bytes = |f: &[f64]| -> Vec<u8> { f.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect() };
                    let mut row = Vec::with_capacity(wu * hu * 12);
                    let (e, t, r) = (bytes(&ours), bytes(today), bytes(&reference));
                    for y in 0..hu {
                        for img in [&e, &t, &r] {
                            row.extend_from_slice(&img[y * wu * 4..(y + 1) * wu * 4]);
                        }
                    }
                    support::write_png(&dir.join(format!("{id}.png")), w * 3, h, &row);
                }
            }
            let stroked = scene.items.iter().any(|i| matches!(i.style, ItemStyle::Stroke(_)));
            for (i, class) in classes.iter().enumerate() {
                if *class != Class::Edge {
                    continue;
                }
                let at = |f: &[f64]| f[i * 4..i * 4 + 4].to_vec();
                let e = cv(&at(&ours), &at(&reference));
                let t = today.as_ref().map(|t| cv(&at(t), &at(&reference)));
                for pool in [Some(&mut all_pool), stroked.then_some(&mut stroked_pool)].into_iter().flatten() {
                    pool.engine.push(e);
                    pool.today.extend(t);
                }
            }
        }
    }
    let show = |name: &str, p: Pool| {
        eprintln!(
            "{name}: engine {} | today {}",
            Stats::of(p.engine),
            Stats::of(p.today)
        );
    };
    eprintln!("{compared} cases compared with today");
    show("edge pixels, all cases (CV vs reference)", all_pool);
    show("edge pixels, stroked cases (CV vs reference)", stroked_pool);
    assert!(
        capture.is_none() || compared > 450 || std::env::var_os("PM_GATE_ONLY").is_some(),
        "the capture covers the cases"
    );
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
