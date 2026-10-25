//! G-V2: bezier path mask coverage against the f64 reference
//! (docs/engine/gate-policy.md; docs/engine/composite.md "Path masks").
//!
//! The cases (`fixtures/mask-cases.json`) are the sweep today's coverage
//! canvases were captured on (`masks-*` in the private corpus, "today"):
//! feathers from 0 to 160, odd sizes and fractional placement, masks
//! crossing and far beyond the canvas edges, rotated and scaled worlds,
//! add, subtract and subtract-only combinations, 10 parametric masks with a
//! path, signed rectangles, HD and UHD.
//!
//! The reference (`support/mask_reference.rs`, shared with the corpus
//! gates) composites, per mask in order, `pm_reference::coverage::
//! fill(outline, NonZero, 16)` feathered by `pm_reference::gaussian::blur`
//! (transparent edges) at the authored σ = max(0, feather)·|world x|/2,
//! `source-over` for add and `destination-out` for subtract, from 1 when no
//! mask adds. Pixels with partial reference coverage, grown by 2 px, are
//! edges; the rest interior.
//!
//! The gate: the engine within its precision bound of the reference
//! everywhere (`ENGINE_BOUND`); against today's capture, the two-part rule
//! (`Report::failures` with `Bounds::EIGHT_BIT`: today's captures are 8-bit);
//! geometry exact where nothing is feathered (alpha area and centroid
//! within 1e-3); and the same canvas from repeated renders (the preview and
//! the export share it). Every case requires its captured legacy witness.
//!
//! `PM_MASK_REF` picks a capture, `PM_MASK_ONLY` a case-id prefix,
//! `PM_MASK_EVIDENCE=<dir>` writes the per-case statistics as JSON there.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};

#[path = "support/mask_reference.rs"]
#[allow(dead_code)]
mod mask_reference;

use pm_reference::compare::{
    Bounds, Class, Stats, classify, compare, geometry, identical, same_geometry,
};
use pm_render::composite::path_masks::{PathCoverage, coverage_values};
use pm_vector::{LayerMasks, MaskMode, MaskShape, MaskValues, PathValues, Vertex};

/// The engine's precision bound per pixel (composite.md "Path masks"): the
/// `r16float` store, half a step below 1 (2⁻¹²), once; per mask, the node
/// interpolation (≤ 2⁻¹²) and kernel tails (1.4·10⁻⁵), and the outline:
/// flattening (10⁻⁴ px), fixed point (2·3·10⁻⁵) and rounded shares
/// (8·10⁻⁵). Errors of successive masks add at most (the composite's
/// partial derivatives are within [0, 1]).
fn engine_bound(masks: usize) -> f64 {
    let store = 2f64.powi(-12);
    let per_mask = 2f64.powi(-12) + 1.4e-5 + 1e-4 + 6e-5 + 8e-5;
    255.0 * (store + masks as f64 * per_mask)
}

struct Case {
    id: String,
    width: u32,
    height: u32,
    world: [f64; 6],
    masks: Vec<MaskValues>,
}

fn cases() -> Vec<Case> {
    let v: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/mask-cases.json")).unwrap();
    let num = |v: &serde_json::Value| v.as_f64().unwrap_or(f64::NAN);
    v["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| Case {
            id: c["id"].as_str().unwrap().to_owned(),
            width: c["width"].as_u64().unwrap() as u32,
            height: c["height"].as_u64().unwrap() as u32,
            world: std::array::from_fn(|i| num(&c["world"][i])),
            masks: c["masks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| {
                    let p = &m["p"];
                    let path = m.get("path").filter(|q| !q.is_null()).map(|q| PathValues {
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
                .collect(),
        })
        .collect()
}

/// `PM_MASK_REF`, or the newest `masks-*` capture holding every case.
fn capture_dir(ids: &[String]) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PM_MASK_REF") {
        return Some(PathBuf::from(dir));
    }
    let root = Path::new(&std::env::var("HOME").ok()?)
        .join("Library/Application Support/Powermove Engine Corpus");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| Some(e.ok()?.path()))
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

/// A captured canvas's coverage (its alpha), 0…1.
fn read_coverage(path: &Path) -> Vec<f64> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    buf.chunks_exact(4)
        .map(|p| f64::from(p[3]) / 255.0)
        .collect()
}

/// Coverage as premultiplied white RGBA.
fn rgba(c: &[f64]) -> Vec<f64> {
    c.iter().flat_map(|v| [*v; 4]).collect()
}

fn stats_json(s: &Stats) -> serde_json::Value {
    serde_json::json!({"pixels": s.pixels, "max": s.max, "p999": s.p999, "mean": s.mean})
}

#[test]
fn path_mask_coverage_gate() {
    let gpu = pm_gpu::Gpu::new("path mask gate").expect("a Metal device");
    let cases = cases();
    let ids: Vec<String> = cases.iter().map(|c| c.id.clone()).collect();
    let capture = capture_dir(&ids).expect("today's complete masks-* witness is required");
    eprintln!("today: {}", capture.display());
    let only = std::env::var("PM_MASK_ONLY").ok();
    let evidence = std::env::var("PM_MASK_EVIDENCE").ok().map(PathBuf::from);
    let mut failures = Vec::new();
    let mut rows = Vec::new();
    for c in cases
        .iter()
        .filter(|c| only.as_deref().is_none_or(|o| c.id.starts_with(o)))
    {
        let LayerMasks::Path(masks) = LayerMasks::new(&c.masks) else {
            panic!("{}: not a path route", c.id)
        };
        let (w, h) = (c.width as usize, c.height as usize);
        let cov = PathCoverage {
            width: c.width,
            height: c.height,
            world: c.world,
            masks: masks.clone(),
        };
        let first = coverage_values(&gpu.device, &gpu.queue, &cov).unwrap();
        let again = coverage_values(&gpu.device, &gpu.queue, &cov).unwrap();
        if let Err(e) = identical(&first, &again) {
            failures.push(format!("{}: repeated renders differ: {e}", c.id));
        }
        let engine: Vec<f64> = first.iter().map(|v| f64::from(*v)).collect();
        let exact = mask_reference::path_coverage(&masks, c.world, w, h);
        let edge: Vec<bool> = exact.iter().map(|v| *v > 1e-9 && *v < 1.0 - 1e-9).collect();
        let classes = classify(&edge, &vec![false; w * h], w, h, 2);
        let today = read_coverage(&capture.join(format!("{}.png", c.id)));
        let report = compare(&rgba(&engine), &rgba(&today), &rgba(&exact), &classes);
        let bound = engine_bound(masks.masks.len());
        let worst = [Class::Interior, Class::Edge]
            .map(|k| report.class(k).engine_vs_reference.max)
            .into_iter()
            .fold(0.0, f64::max);
        if worst > bound {
            failures.push(format!(
                "{}: {worst:.3} CV from the reference (bound {bound:.3})",
                c.id
            ));
        }
        for f in report.failures(&Bounds::EIGHT_BIT) {
            failures.push(format!("{}: {f}", c.id));
        }
        if masks.masks.iter().all(|m| m.feather == 0.0) {
            let (g, r) = (geometry(&engine, w), geometry(&exact, w));
            if let Err(e) = same_geometry(&g, &r, 1e-3) {
                failures.push(format!("{}: geometry {e}", c.id));
            }
        }
        let e = report.class(Class::Edge);
        let sigma = c.masks.iter().map(|m| m.feather).fold(0.0, f64::max)
            * c.world[0].hypot(c.world[1])
            / 2.0;
        eprintln!(
            "{:<26} σ {:>6.2}  edge {:>8} px  engine {}  today {}",
            c.id, sigma, e.engine_vs_reference.pixels, e.engine_vs_reference, e.today_vs_reference,
        );
        rows.push(serde_json::json!({
            "id": c.id,
            "sigma": sigma,
            "masks": masks.masks.len(),
            "bound_cv": bound,
            "edge": {
                "engine_vs_reference": stats_json(&e.engine_vs_reference),
                "today_vs_reference": stats_json(&e.today_vs_reference),
                "excess": stats_json(&e.excess),
            },
            "interior_vs_today": stats_json(&report.class(Class::Interior).engine_vs_today),
        }));
        if let Some(dir) = &evidence {
            std::fs::create_dir_all(dir).unwrap();
            let file = std::fs::File::create(dir.join(format!("{}.png", c.id))).unwrap();
            let mut encoder =
                png::Encoder::new(std::io::BufWriter::new(file), c.width * 3, c.height);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            let mut row = Vec::with_capacity(w * h * 3);
            for y in 0..h {
                for pixels in [&engine, &today, &exact] {
                    row.extend(
                        pixels[y * w..(y + 1) * w]
                            .iter()
                            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8),
                    );
                }
            }
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&row)
                .unwrap();
        }
    }
    if let Some(dir) = evidence {
        std::fs::create_dir_all(&dir).unwrap();
        let doc = serde_json::json!({"capture": capture.display().to_string(), "cases": rows});
        std::fs::write(
            dir.join("g-v2.json"),
            serde_json::to_string_pretty(&doc).unwrap(),
        )
        .unwrap();
    }
    assert!(failures.is_empty(), "G-V2:\n{}", failures.join("\n"));
}
