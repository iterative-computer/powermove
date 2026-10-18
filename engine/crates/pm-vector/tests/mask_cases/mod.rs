//! Path mask coverage cases (`tools/mask-ref`): the case list and the
//! capture directory. Shared with pm-render's GPU coverage test.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use pm_vector::{MaskMode, MaskShape, MaskValues, PathValues, Vertex};

pub struct Case {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub world: [f64; 6],
    pub masks: Vec<MaskValues>,
}

fn num(v: &serde_json::Value) -> f64 {
    v.as_f64().unwrap_or(f64::NAN)
}

pub fn cases() -> Vec<Case> {
    let text = include_str!("../fixtures/mask-cases.json");
    let v: serde_json::Value = serde_json::from_str(text).unwrap();
    v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let w = c["world"].as_array().unwrap();
            let world = [0, 1, 2, 3, 4, 5].map(|i| num(&w[i]));
            let masks = c["masks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| {
                    let p = &m["p"];
                    let path = (!m["path"].is_null()).then(|| {
                        let q = &m["path"];
                        PathValues {
                            x: num(&q["x"]),
                            y: num(&q["y"]),
                            rotation: num(&q["rotation"]),
                            scale_x: num(&q["scaleX"]),
                            scale_y: num(&q["scaleY"]),
                            closed: true,
                            vertices: q["vertices"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|v| Vertex {
                                    x: num(&v["x"]),
                                    y: num(&v["y"]),
                                    in_x: num(&v["inX"]),
                                    in_y: num(&v["inY"]),
                                    out_x: num(&v["outX"]),
                                    out_y: num(&v["outY"]),
                                })
                                .collect(),
                            ..PathValues::default()
                        }
                    });
                    MaskValues {
                        x: num(&p["x"]),
                        y: num(&p["y"]),
                        w: num(&p["w"]),
                        h: num(&p["h"]),
                        rotation: num(&p["rotation"]),
                        feather: num(&p["feather"]),
                        shape: MaskShape::from_name(m["shape"].as_str().unwrap()),
                        mode: MaskMode::from_name(m["mode"].as_str().unwrap()),
                        path,
                    }
                })
                .collect();
            Case {
                id: c["id"].as_str().unwrap().to_owned(),
                width: c["width"].as_u64().unwrap() as u32,
                height: c["height"].as_u64().unwrap() as u32,
                world,
                masks,
            }
        })
        .collect()
}

/// `PM_MASK_REF`, or the newest complete `masks-*` capture.
pub fn capture_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PM_MASK_REF") {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").ok()?;
    let root = Path::new(&home).join("Library/Application Support/Powermove Engine Corpus");
    let ids: Vec<String> = cases().into_iter().map(|c| c.id).collect();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("masks-"))
                && ids.iter().all(|id| p.join(format!("{id}.png")).is_file())
        })
        .collect();
    dirs.sort();
    dirs.pop()
}

/// A reference's coverage (its alpha channel).
pub fn read_coverage(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (
        info.width,
        info.height,
        buf.chunks_exact(4).map(|p| p[3]).collect(),
    )
}

/// (pixels off by more than 1, largest difference, pixels off at all).
pub fn diff(a: &[u8], b: &[u8]) -> (usize, i32, usize) {
    let (mut over1, mut max, mut any) = (0, 0, 0);
    for (x, y) in a.iter().zip(b) {
        let d = (i32::from(*x) - i32::from(*y)).abs();
        over1 += usize::from(d > 1);
        any += usize::from(d > 0);
        max = max.max(d);
    }
    (over1, max, any)
}
