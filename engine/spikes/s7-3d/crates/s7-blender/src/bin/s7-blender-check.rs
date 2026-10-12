//! S7 Blender checks.
//!
//!   s7-blender-check agx <dir> <engine>           AgX + straight alpha vs OCIO expectations
//!   s7-blender-check batch <scene-tag-dir>        batched vs per-layer invocation equality
//!   s7-blender-check orient <blender.png> <three.png>   appendix A item 39 (texture orientation)
//!   s7-blender-check ortho <blender-dir> <three-ref-dir> <layer ...>   item 38 (ortho framing)
//!   s7-blender-check diff <a.png> <b.png>         pixel difference of two surfaces
use anyhow::{Context, Result, bail};
use s7_blender::{Surface16, read};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let out = match a.first().map(String::as_str) {
        Some("agx") => agx(Path::new(&a[1]), &a[2])?,
        Some("batch") => batch(Path::new(&a[1]))?,
        Some("orient") => orient(&read(Path::new(&a[1]))?, &read(Path::new(&a[2]))?)?,
        Some("ortho") => ortho(Path::new(&a[1]), Path::new(&a[2]), &a[3..])?,
        Some("diff") => {
            let (x, y) = (read(Path::new(&a[1]))?, read(Path::new(&a[2]))?);
            let (differ, max) = compare(&x, &y);
            json!({ "channel_values_differ": differ, "max_code_diff": max })
        }
        _ => bail!("usage: s7-blender-check agx|batch|orient|ortho ..."),
    };
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn agx(dir: &Path, engine: &str) -> Result<Value> {
    let img = read(&dir.join(format!("agx-{engine}.png")))?;
    let meta: Value = serde_json::from_slice(&std::fs::read(dir.join(format!("agx-{engine}.json")))?)?;
    let (mut straight, mut premul, mut alpha) = (0i64, 0i64, 0i64);
    let mut rows = Vec::new();
    for p in meta["patches"].as_array().context("patches")? {
        let (x, y) = (p["px"][0].as_u64().unwrap() as u32, p["px"][1].as_u64().unwrap() as u32);
        let m = img.px(x, y);
        let es: Vec<i64> = p["expected_straight"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        let ep: Vec<i64> = p["expected_premul_transform"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
        let ea = p["expected_alpha"].as_i64().unwrap();
        let ds = (0..3).map(|c| (m[c] as i64 - es[c]).abs()).max().unwrap();
        let dp = (0..3).map(|c| (m[c] as i64 - ep[c]).abs()).max().unwrap();
        let da = (m[3] as i64 - ea).abs();
        straight = straight.max(ds);
        premul = premul.max(dp);
        alpha = alpha.max(da);
        rows.push(json!({ "value": p["value"], "tint": p["tint"], "alpha": p["alpha"], "measured": m, "straight": es, "premul_transform": ep, "d_straight": ds, "d_premul": dp, "d_alpha": da }));
    }
    Ok(json!({ "engine": engine, "bit_depth": img.bit_depth, "chunks": img.chunks, "blender": meta["blender"], "ocio": meta["ocio"], "max_code_diff_vs_straight_agx": straight, "max_code_diff_vs_premultiplied_transform": premul, "max_alpha_code_diff": alpha, "patches": rows }))
}

fn compare(a: &Surface16, b: &Surface16) -> (usize, u16) {
    let mut differ = 0;
    let mut max = 0u16;
    for (x, y) in a.rgba.iter().zip(&b.rgba) {
        let d = x.abs_diff(*y);
        if d > 0 {
            differ += 1;
            max = max.max(d);
        }
    }
    (differ, max)
}

fn batch(dir: &Path) -> Result<Value> {
    let manifest: Value = serde_json::from_slice(&std::fs::read(dir.join("batched/manifest.json"))?)?;
    let mut layers = Vec::new();
    let mut equal = true;
    for f in manifest["files"].as_array().context("files")? {
        let id = f["id"].as_str().unwrap();
        let name = f["name"].as_str().unwrap();
        let batched = read(&dir.join("batched").join(name))?;
        let single_path: PathBuf = dir.join(format!("single-{id}")).join(name);
        if !single_path.exists() {
            continue;
        }
        let single = read(&single_path)?;
        let (differ, max) = compare(&batched, &single);
        equal &= differ == 0;
        let bytes_equal = std::fs::read(dir.join("batched").join(name))? == std::fs::read(&single_path)?;
        layers.push(json!({ "layer": id, "bit_depth": batched.bit_depth, "channel_values_differ": differ, "max_code_diff": max, "file_bytes_equal": bytes_equal }));
    }
    Ok(json!({ "dir": dir.display().to_string(), "pixels_equal": equal, "layers": layers }))
}

/// Mean RGB (8-bit scale) of the four quadrants of a box, as TL, TR, BL, BR names.
fn quadrants(img: &Surface16, b: (u32, u32, u32, u32)) -> Vec<String> {
    let (x0, y0, x1, y1) = b;
    let (mx, my) = ((x0 + x1) / 2, (y0 + y1) / 2);
    let boxes = [(x0, y0, mx, my), (mx, y0, x1, my), (x0, my, mx, y1), (mx, my, x1, y1)];
    boxes
        .iter()
        .map(|&(a, b, c, d)| {
            // Sample an inner patch away from the arrow (outer third of each quadrant).
            let (ix0, iy0, ix1, iy1) = (a + (c - a) / 8, b + (d - b) / 8, a + (c - a) * 3 / 8, b + (d - b) * 3 / 8);
            let (ix0, ix1) = if a == x0 { (ix0, ix1) } else { (c - (c - a) * 3 / 8, c - (c - a) / 8) };
            let mut s = [0f64; 3];
            let mut n = 0.0;
            for y in iy0..=iy1 {
                for x in ix0..=ix1 {
                    let p = img.px(x, y);
                    for k in 0..3 {
                        s[k] += p[k] as f64 / 257.0;
                    }
                    n += 1.0;
                }
            }
            let (r, g, bl) = (s[0] / n, s[1] / n, s[2] / n);
            let name = if r > 1.4 * bl && g > 1.4 * bl && (r - g).abs() < 0.35 * r.max(g) {
                "yellow"
            } else if r >= g && r >= bl {
                "red"
            } else if g >= r && g >= bl {
                "green"
            } else {
                "blue"
            };
            format!("{name} ({r:.0},{g:.0},{bl:.0})")
        })
        .collect()
}

fn orient(blender: &Surface16, three: &Surface16) -> Result<Value> {
    // The orientation chart is the left card: the alpha bbox of the left half.
    let left = |img: &Surface16| -> Option<(u32, u32, u32, u32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for y in 0..img.height {
            for x in 0..img.width / 2 {
                if img.px(x, y)[3] > 60000 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        (x1 > x0).then_some((x0, y0, x1, y1))
    };
    let bb = left(blender).context("blender card")?;
    let tb = left(three).context("three card")?;
    let qb = quadrants(blender, bb);
    let qt = quadrants(three, tb);
    let first = |v: &Vec<String>| v.iter().map(|s| s.split(' ').next().unwrap().to_string()).collect::<Vec<_>>();
    let (fb, ft) = (first(&qb), first(&qt));
    let flipped_v = fb == vec![ft[2].clone(), ft[3].clone(), ft[0].clone(), ft[1].clone()];
    Ok(json!({ "quadrants_tl_tr_bl_br": { "three": qt, "blender": qb }, "blender_bbox": bb, "three_bbox": tb, "same_orientation": fb == ft, "blender_is_vertically_flipped": flipped_v }))
}

fn ortho(blender_dir: &Path, three_dir: &Path, layers: &[String]) -> Result<Value> {
    let mut out = Vec::new();
    for l in layers {
        let b = read(&blender_dir.join(format!("{l}.png")))?;
        let t = read(&three_dir.join(format!("layer-{l}.png")))?;
        let (bb, tb) = (b.alpha_bbox().context("blender bbox")?, t.alpha_bbox().context("three bbox")?);
        let size = |q: (u32, u32, u32, u32)| ((q.2 - q.0 + 1) as f64, (q.3 - q.1 + 1) as f64);
        let center = |q: (u32, u32, u32, u32), w: u32, h: u32| ((q.0 + q.2) as f64 / 2.0 - w as f64 / 2.0, (q.1 + q.3) as f64 / 2.0 - h as f64 / 2.0);
        let (bs, ts) = (size(bb), size(tb));
        let (bc, tc) = (center(bb, b.width, b.height), center(tb, t.width, t.height));
        out.push(json!({ "layer": l, "blender_bbox": bb, "three_bbox": tb, "size_ratio_blender_over_three": [bs.0 / ts.0, bs.1 / ts.1], "center_offset_from_frame_center": { "blender": [bc.0, bc.1], "three": [tc.0, tc.1] }, "center_ratio": [if tc.0.abs() > 4.0 { bc.0 / tc.0 } else { f64::NAN }, if tc.1.abs() > 4.0 { bc.1 / tc.1 } else { f64::NAN }] }));
    }
    Ok(json!({ "layers": out }))
}
