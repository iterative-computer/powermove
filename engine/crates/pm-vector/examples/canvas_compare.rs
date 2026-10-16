//! Compares the compat level 1 rasterizer with Chromium's Canvas2D
//! references (tools/canvas-ref). Development tool:
//!   cargo run -p pm-vector --features render --example canvas_compare -- <capture dir> [filter]
use std::path::Path;

use pm_vector::canvas2d::shape::{PathColors, ShapeColors, raster_paths, raster_shape};
use pm_vector::path::{PathLayer, PathNode, PathValues, Vertex};
use pm_vector::shape::{ShapeContent, ShapeKind};

fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

fn num(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::Number(n) => n.as_f64().unwrap(),
        serde_json::Value::Null => 0.0,
        serde_json::Value::Bool(b) => f64::from(u8::from(*b)),
        _ => f64::NAN,
    }
}

fn opt(v: &serde_json::Value) -> Option<f64> {
    if v.is_null() { None } else { Some(num(v)) }
}

fn s(v: &serde_json::Value) -> String {
    v.as_str().map(str::to_owned).unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(&args[1]);
    let filter = args.get(2).cloned().unwrap_or_default();
    let flip = std::env::var("FLIP").is_ok();
    let show = std::env::var("SHOW").is_ok();
    let fixture: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/canvas-cases.json")).unwrap(),
    )
    .unwrap();
    let (mut exact, mut within1, mut worse, mut unsupported) = (0, 0, 0, 0);
    for c in fixture["cases"].as_array().unwrap() {
        let id = c["id"].as_str().unwrap();
        if !id.contains(&filter) {
            continue;
        }
        let path = dir.join(format!("{id}.png"));
        if std::fs::metadata(&path).is_err() {
            continue;
        }
        let result = if c["kind"] == "shape" {
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
        };
        let (w, h, ours) = match result {
            Ok(r) => r,
            Err(e) => {
                unsupported += 1;
                if show { println!("{id:44} {e}"); }
                continue;
            }
        };
        let (rw, rh, mut reference) = read_png(&path);
        if flip {
            let row = (rw * 4) as usize;
            let mut f = vec![0; reference.len()];
            for y in 0..rh as usize {
                f[y * row..(y + 1) * row].copy_from_slice(&reference[(rh as usize - 1 - y) * row..(rh as usize - y) * row]);
            }
            reference = f;
        }
        if (rw, rh) != (w, h) {
            println!("{id:44} SIZE ours {w}x{h} ref {rw}x{rh}");
            worse += 1;
            continue;
        }
        if let Ok(row) = std::env::var("ROW") {
            let y: u32 = row.parse().unwrap();
            let a: Vec<u8> = (0..w).map(|x| ours[((y * w + x) * 4 + 3) as usize]).collect();
            let b: Vec<u8> = (0..w).map(|x| reference[((y * w + x) * 4 + 3) as usize]).collect();
            println!("ours {:?}\nref  {:?}", &a[..a.len().min(40)], &b[..b.len().min(40)]);
            let col: u32 = std::env::var("COL").ok().and_then(|c| c.parse().ok()).unwrap_or(w / 2);
            let a: Vec<u8> = (0..h).map(|y| ours[((y * w + col) * 4 + 3) as usize]).collect();
            let b: Vec<u8> = (0..h).map(|y| reference[((y * w + col) * 4 + 3) as usize]).collect();
            println!("col {col} ours {:?}\ncol {col} ref  {:?}", a, b);
        }
        let (mut diff, mut over1, mut max) = (0usize, 0usize, 0i32);
        let mut shown = 0;
        for (i, (a, b)) in ours.iter().zip(&reference).enumerate() {
            let e = (i32::from(*a) - i32::from(*b)).abs();
            if e > 1 && i % 4 == 3 && shown < 8 && std::env::var("PIX").is_ok() {
                let p = i / 4;
                println!("   pixel ({}, {}) ours {:?} ref {:?}", p as u32 % w, p as u32 / w, &ours[p * 4..p * 4 + 4], &reference[p * 4..p * 4 + 4]);
                shown += 1;
            }
            diff += usize::from(e > 0);
            over1 += usize::from(e > 1);
            max = max.max(e);
        }
        match max {
            0 => exact += 1,
            1 => within1 += 1,
            _ => worse += 1,
        }
        if max > 0 || show {
            println!("{id:44} {w}x{h} differing {diff}, >1 {over1}, max {max}");
        }
    }
    println!("exact {exact}, within 1 {within1}, worse {worse}, unsupported {unsupported}");
}
