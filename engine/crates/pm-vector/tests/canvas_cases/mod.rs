//! The Canvas2D reference cases (`tools/canvas-ref/cases.ts`, written to
//! `tests/fixtures/canvas-cases.json`) replayed on the emulated context, and
//! the capture they are compared with. Shared by `tests/canvas2d.rs` and
//! `examples/canvas_compare.rs`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use pm_vector::canvas2d::shape::{PathColors, RasterError, ShapeColors, raster_paths, raster_shape};
use pm_vector::path::{PathLayer, PathNode, PathValues, Vertex};
use pm_vector::shape::{ShapeContent, ShapeKind};

/// The case list.
pub fn cases() -> Vec<serde_json::Value> {
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/canvas-cases.json")).unwrap(),
    )
    .unwrap();
    fixture["cases"].as_array().unwrap().clone()
}

/// The capture named by `PM_CANVAS_REF` or the newest complete `canvas-*`
/// capture under `~/Library/Application Support/Powermove Engine Corpus/`.
pub fn capture_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PM_CANVAS_REF") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").ok()?;
    let root = Path::new(&home).join("Library/Application Support/Powermove Engine Corpus");
    let ids: Vec<String> = cases().iter().map(|c| c["id"].as_str().unwrap().to_owned()).collect();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("canvas-"))
                && ids.iter().all(|id| p.join(format!("{id}.png")).is_file())
        })
        .collect();
    dirs.sort();
    dirs.pop()
}

pub fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

pub fn num(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap(),
        serde_json::Value::Null => 0.0,
        serde_json::Value::Bool(b) => f64::from(u8::from(*b)),
        _ => f64::NAN,
    }
}

pub fn opt(v: &serde_json::Value) -> Option<f64> {
    if v.is_null() { None } else { Some(num(v)) }
}

pub fn s(v: &serde_json::Value) -> String {
    v.as_str().map(str::to_owned).unwrap_or_default()
}

/// Replays a raw case's canvas calls on the emulated context.
pub fn raw(c: &serde_json::Value) -> (u32, u32, Vec<u8>) {
    use pm_vector::canvas2d::context::Context;
    let (w, h) = (num(&c["width"]) as u32, num(&c["height"]) as u32);
    let mut ctx = Context::new(w, h);
    for op in c["ops"].as_array().unwrap() {
        let op = op.as_array().unwrap();
        let a: Vec<f64> = op[1..].iter().map(num).collect();
        match op[0].as_str().unwrap() {
            "=fillStyle" => ctx.set_fill_style(op[1].as_str().unwrap()),
            "=strokeStyle" => ctx.set_stroke_style(op[1].as_str().unwrap()),
            "=lineWidth" => ctx.set_line_width(a[0]),
            "scale" => ctx.scale(a[0], a[1]),
            "translate" => ctx.translate(a[0], a[1]),
            "rotate" => ctx.rotate(a[0]),
            "beginPath" => ctx.begin_path(),
            "moveTo" => ctx.path.move_to(a[0], a[1]),
            "lineTo" => ctx.path.line_to(a[0], a[1]),
            "bezierCurveTo" => ctx.path.bezier_curve_to(a[0], a[1], a[2], a[3], a[4], a[5]),
            "ellipse" => {
                assert_eq!(a[4], 0.0, "the diagnostics draw unrotated ellipses");
                ctx.path.ellipse(a[0], a[1], a[2], a[3], a[5], a[6], false).unwrap();
            }
            "closePath" => ctx.path.close_path(),
            "fill" => ctx.fill().unwrap(),
            "stroke" => ctx.stroke().unwrap(),
            other => panic!("raw op {other}"),
        }
    }
    (w, h, ctx.finish())
}


/// A rasterized case: width, height, premultiplied RGBA8.
pub type Raster = (u32, u32, Vec<u8>);

/// A case rasterized by the emulation: `None` for SVG cases (rendered by
/// the SVG path, not the Canvas2D emulation).
pub fn rasterize(c: &serde_json::Value) -> Option<Result<Raster, RasterError>> {
    if c["kind"] == "svg" {
        return None;
    }
    Some(if c["kind"] == "raw" {
        Ok(raw(c))
    } else if c["kind"] == "shape" {
        let d = &c["d"];
        let content = ShapeContent {
            shape: ShapeKind::from_name(d["shape"].as_str().unwrap_or("")),
            w: num(&d["w"]),
            h: num(&d["h"]),
            radius: if d.get("radius").is_some() { num(&d["radius"]) } else { f64::NAN },
            independent_corners: d["independentCorners"].as_bool().unwrap_or(false),
            corner_radii: ["radiusTL", "radiusTR", "radiusBR", "radiusBL"].map(|k| d.get(k).and_then(opt)),
            smoothing: if d.get("smoothing").is_some() { num(&d["smoothing"]) } else { f64::NAN },
            stroke: if d.get("stroke").is_some() { num(&d["stroke"]) } else { f64::NAN },
            points: if d.get("points").is_some() { num(&d["points"]) } else { f64::NAN },
        };
        let colors = ShapeColors { color: s(&d["color"]), stroke_color: s(&d["strokeColor"]) };
        raster_shape(&content, &colors, num(&c["scale"])).map(|b| (b.width, b.height, b.rgba))
    } else {
        let mut nodes = vec![];
        let mut colors = vec![];
        for p in c["paths"].as_array().unwrap() {
            let v = &p["p"];
            let n = |k: &str| num(&v[k]);
            nodes.push(PathNode {
                id: s(&p["id"]),
                parent: p["parent"].as_str().map(str::to_owned),
                values: PathValues {
                    x: n("x"), y: n("y"), rotation: n("rotation"), scale_x: n("scaleX"), scale_y: n("scaleY"),
                    closed: v["closed"].as_bool().unwrap_or(false), fill_enabled: v["fillEnabled"].as_bool().unwrap_or(false),
                    fill_opacity: n("fillOpacity"), stroke_width: n("strokeWidth"), stroke_opacity: n("strokeOpacity"),
                    trim_start: n("trimStart"), trim_end: n("trimEnd"), trim_offset: n("trimOffset"), copies: n("copies"),
                    repeat_x: n("repeatX"), repeat_y: n("repeatY"), repeat_rotation: n("repeatRotation"),
                    vertices: p["vertices"].as_array().unwrap().iter().map(|q| Vertex {
                        x: num(&q["x"]), y: num(&q["y"]),
                        in_x: q.get("inX").map_or(0.0, num), in_y: q.get("inY").map_or(0.0, num),
                        out_x: q.get("outX").map_or(0.0, num), out_y: q.get("outY").map_or(0.0, num),
                    }).collect(),
                },
            });
            colors.push(PathColors { fill: s(&v["fill"]), stroke: s(&v["stroke"]) });
        }
        let layer = PathLayer::new(&nodes);
        raster_paths(&layer, &colors, num(&c["scale"])).map(|(b, _)| (b.width, b.height, b.rgba))
    })
}
