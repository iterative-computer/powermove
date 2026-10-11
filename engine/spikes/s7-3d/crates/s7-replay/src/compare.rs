//! The section 2 3D bar: per output, at most 2 CV for 99.5% of pixels, at most 12 CV for
//! any pixel outside MSAA edges and shadow penumbrae, per-object SSIM at least 0.99.
//!
//! Classification masks come from the reference run (`ref/page.ts`):
//! * MSAA edges: pixels where the scene rendered without MSAA differs from the MSAA
//!   render, dilated by one pixel.
//! * Penumbrae: pixels whose shadow factor is strictly between 0 and 1 (ShadowMaterial
//!   alpha 1..254), dilated by one pixel.
//! * Object ids: flat id colors without MSAA; index = drawable list index + 1.
//!
//! CV differences are per channel on 8-bit premultiplied RGBA; a pixel's difference is
//! its largest channel difference. SSIM is the mean of the per-channel (R, G, B, A) SSIM
//! maps (11×11 Gaussian, σ = 1.5, K1 = 0.01, K2 = 0.03, L = 255) over an object's pixels.

use crate::image::{Rgba8, read_png};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::path::Path;

pub const MAX_FRACTION_CV: u8 = 2;
pub const FRACTION_TARGET: f64 = 0.995;
pub const MAX_CV_OUTSIDE: u8 = 12;
pub const SSIM_TARGET: f64 = 0.99;

#[derive(Serialize, Clone, Debug)]
pub struct ObjectScore {
    pub id: u32,
    pub owner: String,
    pub name: String,
    pub kind: String,
    pub pixels: usize,
    pub ssim: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct OutputScore {
    pub output: String,
    pub pixels: usize,
    pub fraction_within_2: f64,
    pub max_cv: u8,
    pub max_cv_outside_edges: u8,
    pub excluded_pixels: usize,
    pub over_12_outside: usize,
    pub min_object_ssim: Option<f64>,
    pub objects: Vec<ObjectScore>,
    pub pass_fraction: bool,
    pub pass_max: bool,
    pub pass_ssim: bool,
    pub pass: bool,
}

fn dilate(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = mask.to_vec();
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] {
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                            out[ny as usize * w + nx as usize] = true;
                        }
                    }
                }
            }
        }
    }
    out
}

fn gaussian_blur(src: &[f64], w: usize, h: usize) -> Vec<f64> {
    let sigma = 1.5f64;
    let k: Vec<f64> = (-5i32..=5).map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp()).collect();
    let sum: f64 = k.iter().sum();
    let k: Vec<f64> = k.iter().map(|v| v / sum).collect();
    let mut tmp = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let sx = (x as i32 + i as i32 - 5).clamp(0, w as i32 - 1) as usize;
                acc += kv * src[y * w + sx];
            }
            tmp[y * w + x] = acc;
        }
    }
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let sy = (y as i32 + i as i32 - 5).clamp(0, h as i32 - 1) as usize;
                acc += kv * tmp[sy * w + x];
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// Mean of the four per-channel SSIM maps.
pub fn ssim_map(a: &Rgba8, b: &Rgba8) -> Vec<f64> {
    let (w, h) = (a.width as usize, a.height as usize);
    let c1 = (0.01f64 * 255.0).powi(2);
    let c2 = (0.03f64 * 255.0).powi(2);
    let mut total = vec![0.0; w * h];
    for ch in 0..4 {
        let x: Vec<f64> = (0..w * h).map(|i| a.data[i * 4 + ch] as f64).collect();
        let y: Vec<f64> = (0..w * h).map(|i| b.data[i * 4 + ch] as f64).collect();
        let mx = gaussian_blur(&x, w, h);
        let my = gaussian_blur(&y, w, h);
        let xx = gaussian_blur(&x.iter().map(|v| v * v).collect::<Vec<_>>(), w, h);
        let yy = gaussian_blur(&y.iter().map(|v| v * v).collect::<Vec<_>>(), w, h);
        let xy = gaussian_blur(&x.iter().zip(&y).map(|(p, q)| p * q).collect::<Vec<_>>(), w, h);
        for i in 0..w * h {
            let (sx, sy, sxy) = (xx[i] - mx[i] * mx[i], yy[i] - my[i] * my[i], xy[i] - mx[i] * my[i]);
            let s = ((2.0 * mx[i] * my[i] + c1) * (2.0 * sxy + c2)) / ((mx[i] * mx[i] + my[i] * my[i] + c1) * (sx + sy + c2));
            total[i] += s / 4.0;
        }
    }
    total
}

pub struct SceneMasks {
    pub excluded: Vec<bool>,
    pub ids: Vec<u32>,
    pub drawables: Vec<(u32, String, String, String)>,
    pub width: usize,
    pub height: usize,
}

pub fn load_masks(ref_dir: &Path) -> Result<SceneMasks> {
    let info: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(ref_dir.join("info.json"))?)?;
    let full = read_png(&ref_dir.join("full.png"))?;
    let noaa = read_png(&ref_dir.join("mask-noaa.png"))?;
    let shadow = read_png(&ref_dir.join("mask-shadow.png"))?;
    let ids = read_png(&ref_dir.join("mask-ids.png"))?;
    let (w, h) = (full.width as usize, full.height as usize);
    let msaa: Vec<bool> = (0..w * h).map(|i| (0..4).any(|c| full.data[i * 4 + c] != noaa.data[i * 4 + c])).collect();
    let penumbra: Vec<bool> = (0..w * h).map(|i| (1..=254).contains(&shadow.data[i * 4 + 3])).collect();
    let msaa = dilate(&msaa, w, h);
    let penumbra = dilate(&penumbra, w, h);
    let excluded = msaa.iter().zip(&penumbra).map(|(a, b)| *a || *b).collect();
    let id_values = (0..w * h).map(|i| ids.data[i * 4] as u32 | (ids.data[i * 4 + 1] as u32) << 8 | (ids.data[i * 4 + 2] as u32) << 16).collect();
    let drawables = info["drawables"]
        .as_array()
        .map(|a| a.iter().map(|d| (d["id"].as_u64().unwrap_or(0) as u32, d["owner"].as_str().unwrap_or("").to_string(), d["name"].as_str().unwrap_or("").to_string(), d["kind"].as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    Ok(SceneMasks { excluded, ids: id_values, drawables, width: w, height: h })
}

pub fn score(output: &str, reference: &Rgba8, test: &Rgba8, masks: &SceneMasks) -> Result<(OutputScore, Vec<u8>)> {
    if reference.width != test.width || reference.height != test.height {
        bail!("{output}: size mismatch {}x{} vs {}x{}", reference.width, reference.height, test.width, test.height);
    }
    let n = (reference.width * reference.height) as usize;
    let mut within = 0usize;
    let mut max_cv = 0u8;
    let mut max_outside = 0u8;
    let mut over = 0usize;
    let mut heat = vec![0u8; n * 4];
    for i in 0..n {
        let d = (0..4).map(|c| reference.data[i * 4 + c].abs_diff(test.data[i * 4 + c])).max().unwrap();
        if d <= MAX_FRACTION_CV {
            within += 1;
        }
        max_cv = max_cv.max(d);
        if !masks.excluded[i] {
            max_outside = max_outside.max(d);
            if d > MAX_CV_OUTSIDE {
                over += 1;
            }
        }
        let v = (d as u32 * 16).min(255) as u8;
        heat[i * 4..i * 4 + 4].copy_from_slice(&[v, if masks.excluded[i] { 40 } else { 0 }, 0, 255]);
    }
    // Objects drawn into this output.
    let owner = output.strip_prefix("layer-");
    let ssim = ssim_map(reference, test);
    let mut objects = Vec::new();
    for (id, own, name, kind) in &masks.drawables {
        if let Some(o) = owner {
            if own != o {
                continue;
            }
        }
        let mut sum = 0.0;
        let mut count = 0usize;
        for i in 0..n {
            if masks.ids[i] == *id {
                sum += ssim[i];
                count += 1;
            }
        }
        if count > 0 {
            objects.push(ObjectScore { id: *id, owner: own.clone(), name: name.clone(), kind: kind.clone(), pixels: count, ssim: sum / count as f64 });
        }
    }
    let min_ssim = objects.iter().map(|o| o.ssim).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.min(v))));
    let fraction = within as f64 / n as f64;
    let pass_fraction = fraction >= FRACTION_TARGET;
    let pass_max = max_outside <= MAX_CV_OUTSIDE;
    let pass_ssim = min_ssim.is_none_or(|s| s >= SSIM_TARGET);
    Ok((
        OutputScore {
            output: output.to_string(),
            pixels: n,
            fraction_within_2: fraction,
            max_cv,
            max_cv_outside_edges: max_outside,
            excluded_pixels: masks.excluded.iter().filter(|e| **e).count(),
            over_12_outside: over,
            min_object_ssim: min_ssim,
            objects,
            pass_fraction,
            pass_max,
            pass_ssim,
            pass: pass_fraction && pass_max && pass_ssim,
        },
        heat,
    ))
}

/// Scores every output of one scene. Returns scores in reference output order.
pub fn score_scene(ref_dir: &Path, test_dir: &Path, heat_dir: Option<&Path>) -> Result<Vec<OutputScore>> {
    let info: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(ref_dir.join("info.json"))?)?;
    let masks = load_masks(ref_dir)?;
    let mut scores = Vec::new();
    for o in info["outputs"].as_array().context("outputs")? {
        let name = o["name"].as_str().context("output name")?;
        let reference = read_png(&ref_dir.join(format!("{name}.png")))?;
        let test_path = test_dir.join(format!("{name}.png"));
        if !test_path.exists() {
            bail!("{}: missing", test_path.display());
        }
        let test = read_png(&test_path)?;
        let (s, heat) = score(name, &reference, &test, &masks)?;
        if let Some(dir) = heat_dir {
            std::fs::create_dir_all(dir)?;
            crate::image::write_png(&dir.join(format!("{name}.diff.png")), reference.width, reference.height, &heat)?;
        }
        scores.push(s);
    }
    Ok(scores)
}
