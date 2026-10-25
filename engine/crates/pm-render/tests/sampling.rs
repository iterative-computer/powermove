//! Raster sampling and antialiasing at every compatibility level (plan section 8; section 2's
//! antialiasing bar; docs/engine/motion.md).
//!
//! - `antialiasing_bar_zone_plate`: a zone plate drawn as a `compat` 2
//!   image under animated scale 1.0 → 0.25 and rotation, against a
//!   reference that supersamples the analytic signal 16× per axis and
//!   filters it with the final path's declared kernel (Lanczos-3 in output
//!   pixels): PSNR ≥ 40 dB for export and paused frames (Lanczos), ≥ 34 dB
//!   during playback (trilinear), flicker energy at most 1.5× the
//!   reference's.
//! - Analytic coverage edges against the exact pixel-area coverage,
//!   continuous rasterization of level 2 precomps,
//!   regions and determinism.

mod support;

use std::path::{Path, PathBuf};

use pm_protocol::color::Precision;
use pm_protocol::common::{PixelRect, Size};
use pm_render::sampling::Sampling;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::sampling_reference::lanczos3;
use support::ticks;

/// A temporary directory outside the repository for one test's sources.
struct Scratch(PathBuf);
impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(test: &str) -> Scratch {
    let dir =
        std::env::temp_dir().join(format!("pm-render-sampling-{}-{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

/// Writes an 8-bit RGB PNG.
fn write_rgb(path: &Path, width: u32, height: u32, value: impl Fn(u32, u32) -> f64) {
    let mut data = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        for x in 0..width {
            let v = (value(x, y).clamp(0.0, 1.0) * 255.0).round() as u8;
            data.extend_from_slice(&[v, v, v]);
        }
    }
    let file = std::fs::File::create(path).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&data).unwrap();
}

const SOURCE: u32 = 256;
const COMP: u32 = 384;

/// The zone plate: 0.5 + 0.5 cos(π f r² / R) in texel units from the
/// center, reaching 0.4 cycles per texel at r = R (below Nyquist).
fn zone(u: f64, v: f64) -> f64 {
    let c = f64::from(SOURCE) / 2.0;
    let (dx, dy) = (u - c, v - c);
    let r2 = dx * dx + dy * dy;
    // Phase φ = π k r²: local frequency k r cycles per texel; 0.4 at r = c.
    let k = 0.4 / c;
    0.5 + 0.5 * (std::f64::consts::PI * k * r2).cos()
}

/// An image layer of the source at the composition's center.
fn image_doc(
    path: &Path,
    compat: u32,
    scale: f64,
    rotation: f64,
) -> (pm_model::Document, MapResolver) {
    let mut l = layer(
        "img",
        "image",
        json!({ "asset": "src", "fit": "fill", "w": SOURCE, "h": SOURCE }),
        f64::from(COMP) / 2.0,
        f64::from(COMP) / 2.0,
        4.0,
    );
    set(&mut l, "scale.x", json!(scale * 100.0));
    set(&mut l, "scale.y", json!(scale * 100.0));
    set(&mut l, "rotation", json!(rotation));
    l["compat"] = json!(compat);
    let mut doc = project(COMP, COMP, 4.0, no_fill(), vec![l]);
    asset(&mut doc, "src", "image", "png", SOURCE, SOURCE, 0.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("src".into(), path.to_path_buf());
    (load(&doc), resolver)
}

fn render(
    renderer: &mut Renderer,
    doc: &pm_model::Document,
    resolver: &MapResolver,
    request: &RenderRequest,
) -> Vec<f32> {
    let f = renderer
        .render(doc, "Ptest-root", ticks(0.0), request, resolver)
        .unwrap();
    assert!(f.failures.is_empty(), "{:?}", f.failures);
    renderer.read_rgba(&f).unwrap()
}

/// The reference frame under the final path's declared kernels: the
/// analytic source (extended past its edges as the clamped texel fetch
/// extends it) supersampled 16× per axis in output pixels and filtered by
/// a separable Lanczos-3 of 3 output pixels, times the exact pixel-area
/// coverage of the quad (the declared edge kernel), premultiplied gray
/// with alpha. Only the pixels within `bounds` are computed (the rest is
/// transparent).
fn reference(
    scale: f64,
    rotation: f64,
    bounds: [usize; 4],
    continuous: fn(f64, f64) -> f64,
) -> Vec<f64> {
    const SS: usize = 16;
    let n = COMP as usize;
    let [x0, y0, x1, y1] = bounds;
    // Supersampled rows and columns covering the bounds plus the support.
    let sx0 = x0.saturating_sub(3) * SS;
    let sy0 = y0.saturating_sub(3) * SS;
    let sx1 = ((x1 + 3).min(n)) * SS;
    let sy1 = ((y1 + 3).min(n)) * SS;
    let (sw, sh) = (sx1 - sx0, sy1 - sy0);
    let (sin, cos) = rotation.to_radians().sin_cos();
    let c = f64::from(COMP) / 2.0;
    let half = f64::from(SOURCE) / 2.0;
    // The continuous image: color and coverage at an output point.
    let c0 = c;
    let mut color = vec![0.0f64; sw * sh];
    for j in 0..sh {
        for i in 0..sw {
            let x = (sx0 + i) as f64 / SS as f64 + 0.5 / SS as f64;
            let y = (sy0 + j) as f64 / SS as f64 + 0.5 / SS as f64;
            // Output point to layer pixels: undo the rotation and scale.
            let (dx, dy) = (x - c, y - c);
            let u = (cos * dx + sin * dy) / scale + half;
            let v = (-sin * dx + cos * dy) / scale + half;
            let edge = f64::from(SOURCE) - 0.5;
            color[j * sw + i] = continuous(u.clamp(0.5, edge), v.clamp(0.5, edge));
        }
    }
    // Separable Lanczos-3 at output pixel centers.
    let taps: Vec<f64> = (0..6 * SS)
        .map(|k| lanczos3((k as f64 + 0.5) / SS as f64 - 3.0))
        .collect();
    let norm: f64 = taps.iter().sum();
    let filter = |plane: &[f64]| -> Vec<f64> {
        let mut rows = vec![0.0; (x1 - x0) * sh];
        for j in 0..sh {
            for (ox, px) in (x0..x1).enumerate() {
                let start = px * SS + SS / 2 - 3 * SS;
                let mut s = 0.0;
                for (k, w) in taps.iter().enumerate() {
                    let at = start as isize + k as isize - sx0 as isize;
                    if at >= 0 && (at as usize) < sw {
                        s += w * plane[j * sw + at as usize];
                    }
                }
                rows[j * (x1 - x0) + ox] = s / norm;
            }
        }
        let mut out = vec![0.0; (x1 - x0) * (y1 - y0)];
        for (oy, py) in (y0..y1).enumerate() {
            let start = py * SS + SS / 2 - 3 * SS;
            for ox in 0..(x1 - x0) {
                let mut s = 0.0;
                for (k, w) in taps.iter().enumerate() {
                    let at = start as isize + k as isize - sy0 as isize;
                    if at >= 0 && (at as usize) < sh {
                        s += w * rows[at as usize * (x1 - x0) + ox];
                    }
                }
                out[oy * (x1 - x0) + ox] = s / norm;
            }
        }
        out
    };
    let c = filter(&color);
    let r = f64::from(SOURCE) / 2.0 * scale;
    let corner = |sx: f64, sy: f64| {
        let (x, y) = (sx * r, sy * r);
        [c0 + cos * x - sin * y, c0 + sin * x + cos * y]
    };
    let quad = [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ];
    let mut frame = vec![0.0; n * n * 2];
    for (oy, py) in (y0..y1).enumerate() {
        for (ox, px) in (x0..x1).enumerate() {
            let a = pixel_coverage(&quad, px as f64, py as f64);
            frame[(py * n + px) * 2] = c[oy * (x1 - x0) + ox] * a;
            frame[(py * n + px) * 2 + 1] = a;
        }
    }
    frame
}

/// The final path's declared kernel evaluated exactly in f64 on the same
/// 8-bit texels (`texels`, row-major, `SOURCE`²): Lanczos-3 in the
/// mapping's principal axes (the texel axes for these isotropic maps) with
/// its support scaled by the minification, normalized, clamped to the
/// central texels (anti-ringing), times the exact pixel-area coverage of
/// the quad (the declared edge kernel, the limit of supersampling).
fn declared_reference(scale: f64, rotation: f64, bounds: [usize; 4], texels: &[f64]) -> Vec<f64> {
    let n = COMP as usize;
    let size = SOURCE as i64;
    let (sin, cos) = rotation.to_radians().sin_cos();
    let c = f64::from(COMP) / 2.0;
    let half = f64::from(SOURCE) / 2.0;
    let k = (1.0 / scale).max(1.0);
    let r = half * scale;
    let corner = |sx: f64, sy: f64| {
        let (x, y) = (sx * r, sy * r);
        [c + cos * x - sin * y, c + sin * x + cos * y]
    };
    let quad = [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ];
    let mut frame = vec![0.0; n * n * 2];
    for py in bounds[1]..bounds[3] {
        for px in bounds[0]..bounds[2] {
            let a = pixel_coverage(&quad, px as f64, py as f64);
            if a <= 0.0 {
                continue;
            }
            let (dx, dy) = (px as f64 + 0.5 - c, py as f64 + 0.5 - c);
            let u = ((cos * dx + sin * dy) / scale + half).clamp(0.0, f64::from(SOURCE));
            let v = ((-sin * dx + cos * dy) / scale + half).clamp(0.0, f64::from(SOURCE));
            let (mut sum, mut wsum) = (0.0, 0.0);
            let (mut lo, mut hi) = (f64::MAX, f64::MIN);
            let ext = 3.0 * k;
            for ty in (v - ext).floor() as i64..=(v + ext).ceil() as i64 {
                for tx in (u - ext).floor() as i64..=(u + ext).ceil() as i64 {
                    let kx = (tx as f64 + 0.5 - u) / k;
                    let ky = (ty as f64 + 0.5 - v) / k;
                    if kx.abs() >= 3.0 || ky.abs() >= 3.0 {
                        continue;
                    }
                    let t = texels[(ty.clamp(0, size - 1) * size + tx.clamp(0, size - 1)) as usize];
                    let w = lanczos3(kx) * lanczos3(ky);
                    sum += w * t;
                    wsum += w;
                    if kx.abs() < 1.0 && ky.abs() < 1.0 {
                        lo = lo.min(t);
                        hi = hi.max(t);
                    }
                }
            }
            let value = (sum / wsum).clamp(lo, hi);
            frame[(py * n + px) * 2] = value * a;
            frame[(py * n + px) * 2 + 1] = a;
        }
    }
    frame
}

/// The playback path's declared kernel in f64 on the same texels: the box
/// mip pyramid (`mips.wgsl`), the isotropic level of detail log2 of the
/// longer Jacobian column, trilinear filtering with clamp-to-edge, times
/// the exact pixel-area coverage.
fn trilinear_reference(scale: f64, rotation: f64, bounds: [usize; 4], texels: &[f64]) -> Vec<f64> {
    let n = COMP as usize;
    // The pyramid.
    let mut levels: Vec<(usize, Vec<f64>)> = vec![(SOURCE as usize, texels.to_vec())];
    while levels.last().unwrap().0 > 1 {
        let (w, above) = levels.last().unwrap();
        let (w, nw) = (*w, (*w / 2).max(1));
        let at = |x: usize, y: usize| above[y.min(w - 1) * w + x.min(w - 1)];
        let mut next = vec![0.0; nw * nw];
        for y in 0..nw {
            for x in 0..nw {
                next[y * nw + x] = (at(2 * x, 2 * y)
                    + at(2 * x + 1, 2 * y)
                    + at(2 * x, 2 * y + 1)
                    + at(2 * x + 1, 2 * y + 1))
                    / 4.0;
            }
        }
        levels.push((nw, next));
    }
    let bilinear = |level: usize, u: f64, v: f64| {
        let (w, data) = &levels[level];
        let w = *w;
        let (x, y) = (u * w as f64 - 0.5, v * w as f64 - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let at = |i: f64, j: f64| {
            let (i, j) = (
                (i.max(0.0) as usize).min(w - 1),
                (j.max(0.0) as usize).min(w - 1),
            );
            data[j * w + i]
        };
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
        top * (1.0 - fy) + bottom * fy
    };
    let (sin, cos) = rotation.to_radians().sin_cos();
    let c = f64::from(COMP) / 2.0;
    let half = f64::from(SOURCE) / 2.0;
    let r = half * scale;
    let corner = |sx: f64, sy: f64| {
        let (x, y) = (sx * r, sy * r);
        [c + cos * x - sin * y, c + sin * x + cos * y]
    };
    let quad = [
        corner(-1.0, -1.0),
        corner(1.0, -1.0),
        corner(1.0, 1.0),
        corner(-1.0, 1.0),
    ];
    // Texels per output pixel along either axis (isotropic).
    let lod = (1.0 / scale).log2().clamp(0.0, (levels.len() - 1) as f64);
    let (l0, frac) = (lod.floor() as usize, lod - lod.floor());
    let l1 = (l0 + 1).min(levels.len() - 1);
    let mut frame = vec![0.0; n * n * 2];
    for py in bounds[1]..bounds[3] {
        for px in bounds[0]..bounds[2] {
            let a = pixel_coverage(&quad, px as f64, py as f64);
            if a <= 0.0 {
                continue;
            }
            let (dx, dy) = (px as f64 + 0.5 - c, py as f64 + 0.5 - c);
            let size = f64::from(SOURCE);
            let u = (((cos * dx + sin * dy) / scale + half) / size).clamp(0.0, 1.0);
            let v = (((-sin * dx + cos * dy) / scale + half) / size).clamp(0.0, 1.0);
            let value = bilinear(l0, u, v) * (1.0 - frac) + bilinear(l1, u, v) * frac;
            frame[(py * n + px) * 2] = value * a;
            frame[(py * n + px) * 2 + 1] = a;
        }
    }
    frame
}

/// The output pixel bounds of the rotated, scaled image, grown by 4.
fn quad_bounds(scale: f64, rotation: f64) -> [usize; 4] {
    let (sin, cos) = rotation.to_radians().sin_cos();
    let r = f64::from(SOURCE) / 2.0 * scale * (sin.abs() + cos.abs());
    let c = f64::from(COMP) / 2.0;
    let lo = ((c - r).floor() - 4.0).max(0.0) as usize;
    let hi = (((c + r).ceil() + 4.0) as usize).min(COMP as usize);
    [lo, lo, hi, hi]
}

struct Sequence {
    psnr_min: f64,
    psnr_mean: f64,
    flicker_ratio: f64,
}

/// PSNR (peak 1) of the premultiplied gray and alpha within `bounds`, and
/// flicker: the energy of the second temporal difference over the frames.
fn measure(ours: &[Vec<f32>], refs: &[Vec<f64>], bounds: &[[usize; 4]]) -> Sequence {
    let n = COMP as usize;
    let mut psnrs = Vec::new();
    for ((o, r), b) in ours.iter().zip(refs).zip(bounds) {
        let mut se = 0.0;
        let mut count = 0.0;
        for y in b[1]..b[3] {
            for x in b[0]..b[2] {
                let i = y * n + x;
                for (ch, refch) in [(0usize, 0usize), (3, 1)] {
                    let d = f64::from(o[i * 4 + ch]) - r[i * 2 + refch];
                    se += d * d;
                    count += 1.0;
                }
            }
        }
        psnrs.push(10.0 * (1.0 / (se / count)).log10());
        if std::env::var("PM_AA_FRAMES").is_ok() {
            eprintln!("  frame PSNR {:.2} dB", psnrs.last().unwrap());
        }
    }
    // Flicker over the union of the bounds.
    let ub = bounds.iter().fold([n, n, 0, 0], |u, b| {
        [
            u[0].min(b[0]),
            u[1].min(b[1]),
            u[2].max(b[2]),
            u[3].max(b[3]),
        ]
    });
    let mut e_ours = 0.0;
    let mut e_ref = 0.0;
    for k in 1..ours.len() - 1 {
        for y in ub[1]..ub[3] {
            for x in ub[0]..ub[2] {
                let i = y * n + x;
                let d = f64::from(ours[k + 1][i * 4]) - 2.0 * f64::from(ours[k][i * 4])
                    + f64::from(ours[k - 1][i * 4]);
                e_ours += d * d;
                let d = refs[k + 1][i * 2] - 2.0 * refs[k][i * 2] + refs[k - 1][i * 2];
                e_ref += d * d;
            }
        }
    }
    Sequence {
        psnr_min: psnrs.iter().cloned().fold(f64::MAX, f64::min),
        psnr_mean: psnrs.iter().sum::<f64>() / psnrs.len() as f64,
        flicker_ratio: e_ours / e_ref,
    }
}

/// A Siemens star of 32 sector pairs: 1 in alternate sectors.
fn star(u: f64, v: f64) -> f64 {
    let c = f64::from(SOURCE) / 2.0;
    let phi = (v - c).atan2(u - c);
    if (32.0 * phi).sin() >= 0.0 { 1.0 } else { 0.0 }
}

/// The star's texels: each the 16 × 16 box average of the hard star (an
/// area-sampled source, as a rendered star would be).
fn star_texel(x: u32, y: u32) -> f64 {
    let mut sum = 0.0;
    for j in 0..16 {
        for i in 0..16 {
            sum += star(
                f64::from(x) + (f64::from(i) + 0.5) / 16.0,
                f64::from(y) + (f64::from(j) + 0.5) / 16.0,
            );
        }
    }
    sum / 256.0
}

/// Section 2's antialiasing bar for one pattern.
fn antialiasing_bar(name: &str, texel: &dyn Fn(u32, u32) -> f64, continuous: fn(f64, f64) -> f64) {
    let dir = scratch(&format!("antialiasing_bar_{name}"));
    let path = dir.join("source.png");
    write_rgb(&path, SOURCE, SOURCE, texel);
    let mut renderer = Renderer::headless().unwrap();
    // Scale 1.0 → 0.25 with a quarter turn, in 16 frames.
    let frames: Vec<(f64, f64)> = (0..16)
        .map(|k| {
            let t = f64::from(k) / 15.0;
            (1.0 - 0.75 * t, 90.0 * t)
        })
        .collect();
    let bounds: Vec<[usize; 4]> = frames.iter().map(|(s, r)| quad_bounds(*s, *r)).collect();
    let texels: Vec<f64> = (0..SOURCE * SOURCE)
        .map(|i| (texel(i % SOURCE, i / SOURCE).clamp(0.0, 1.0) * 255.0).round() / 255.0)
        .collect();
    let lanczos: Vec<Vec<f64>> = frames
        .iter()
        .zip(&bounds)
        .map(|((s, r), b)| declared_reference(*s, *r, *b, &texels))
        .collect();
    let trilinear: Vec<Vec<f64>> = frames
        .iter()
        .zip(&bounds)
        .map(|((s, r), b)| trilinear_reference(*s, *r, *b, &texels))
        .collect();
    // For scale: the continuous pattern supersampled 16× and low-passed by
    // Lanczos-3 in output pixels (not a bar).
    let low_pass: Vec<Vec<f64>> = frames
        .iter()
        .zip(&bounds)
        .map(|((s, r), b)| reference(*s, *r, *b, continuous))
        .collect();
    for (compat, sampling, precision, bar) in [
        (1, Sampling::Final, Precision::Float32, 40.0),
        (2, Sampling::Final, Precision::Float32, 40.0),
        (1, Sampling::Final, Precision::Float16, 40.0),
        (2, Sampling::Final, Precision::Float16, 40.0),
        (1, Sampling::Playback, Precision::Float32, 34.0),
        (2, Sampling::Playback, Precision::Float32, 34.0),
        (1, Sampling::Playback, Precision::Float16, 34.0),
        (2, Sampling::Playback, Precision::Float16, 34.0),
    ] {
        let refs = match sampling {
            Sampling::Final => &lanczos,
            Sampling::Playback => &trilinear,
        };
        let ours: Vec<Vec<f32>> = frames
            .iter()
            .map(|(s, r)| {
                let (doc, resolver) = image_doc(&path, compat, *s, *r);
                let mut request = RenderRequest::sized(Size::new(COMP, COMP), precision);
                request.background = false;
                request.sampling = sampling;
                render(&mut renderer, &doc, &resolver, &request)
            })
            .collect();
        let m = measure(&ours, refs, &bounds);
        let i = measure(&ours, &low_pass, &bounds);
        eprintln!(
            "{name} compat {compat} {sampling:?} {precision:?}: against the declared kernel PSNR min {:.2} dB \
             mean {:.2} dB, flicker {:.3}×; against the output-space low-pass PSNR min {:.2} dB mean {:.2} dB",
            m.psnr_min, m.psnr_mean, m.flicker_ratio, i.psnr_min, i.psnr_mean
        );
        assert!(
            m.psnr_min >= bar,
            "{name} {sampling:?}: PSNR {:.2} < {bar}",
            m.psnr_min
        );
        assert!(
            m.flicker_ratio <= 1.5,
            "{name} {sampling:?}: flicker {:.3}",
            m.flicker_ratio
        );
    }
}

#[test]
fn antialiasing_bar_zone_plate() {
    antialiasing_bar(
        "zone plate",
        &|x, y| zone(f64::from(x) + 0.5, f64::from(y) + 0.5),
        zone,
    );
}

#[test]
fn antialiasing_bar_siemens_star() {
    antialiasing_bar("Siemens star", &star_texel, star);
}

/// The exact area of the unit pixel square at (px, py) inside a convex
/// polygon (f64 clipping).
fn pixel_coverage(poly: &[[f64; 2]], px: f64, py: f64) -> f64 {
    let mut p: Vec<[f64; 2]> = vec![
        [px, py],
        [px + 1.0, py],
        [px + 1.0, py + 1.0],
        [px, py + 1.0],
    ];
    let area = |q: &[[f64; 2]]| -> f64 {
        (0..q.len())
            .map(|i| {
                let (a, b) = (q[i], q[(i + 1) % q.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            / 2.0
    };
    let orient = area(poly).signum();
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let side =
            |q: [f64; 2]| orient * ((b[0] - a[0]) * (q[1] - a[1]) - (b[1] - a[1]) * (q[0] - a[0]));
        let mut out = Vec::new();
        for k in 0..p.len() {
            let (c, d) = (p[k], p[(k + 1) % p.len()]);
            let (sc, sd) = (side(c), side(d));
            if sc >= 0.0 {
                out.push(c);
            }
            if (sc >= 0.0) != (sd >= 0.0) {
                let t = sc / (sc - sd);
                out.push([c[0] + (d[0] - c[0]) * t, c[1] + (d[1] - c[1]) * t]);
            }
        }
        p = out;
        if p.is_empty() {
            return 0.0;
        }
    }
    area(&p).abs()
}

#[test]
fn raster_edges_are_the_exact_pixel_coverage_at_every_level() {
    let dir = scratch("level2_edges_are_the_exact_pixel_coverage");
    let path = dir.join("white.png");
    write_rgb(&path, 64, 64, |_, _| 1.0);
    let mut renderer = Renderer::headless().unwrap();
    for (compat, rotation, scale) in [
        (1, 0.0, 1.0),
        (1, 30.0, 1.37),
        (1, -12.5, 0.61),
        (2, 0.0, 1.0),
        (2, 30.0, 1.37),
        (2, -12.5, 0.61),
    ] {
        let mut l = layer(
            "img",
            "image",
            json!({ "asset": "src", "fit": "fill", "w": 64, "h": 64 }),
            100.3,
            80.7,
            4.0,
        );
        set(&mut l, "rotation", json!(rotation));
        set(&mut l, "scale.x", json!(scale * 100.0));
        set(&mut l, "scale.y", json!(scale * 100.0));
        l["compat"] = json!(compat);
        let mut doc = project(200, 160, 4.0, no_fill(), vec![l]);
        // This test's exact edge-coverage bar needs the 32-bit working target.
        doc["colorDepth"] = json!(32);
        asset(&mut doc, "src", "image", "png", 64, 64, 0.0);
        let mut resolver = MapResolver::default();
        resolver.paths.insert("src".into(), path.clone());
        let doc = load(&doc);
        for sampling in [Sampling::Final, Sampling::Playback] {
            let mut request = RenderRequest::sized(Size::new(200, 160), Precision::Float32);
            request.background = false;
            request.sampling = sampling;
            let frame = renderer
                .render(&doc, "Ptest-root", ticks(0.0), &request, &resolver)
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba32Float);
            let rgba = renderer.read_rgba(&frame).unwrap();
            let (sin, cos) = f64::to_radians(rotation).sin_cos();
            let corner = |u: f64, v: f64| {
                let (x, y) = ((u - 32.0) * scale, (v - 32.0) * scale);
                [100.3 + cos * x - sin * y, 80.7 + sin * x + cos * y]
            };
            let poly = [
                corner(0.0, 0.0),
                corner(64.0, 0.0),
                corner(64.0, 64.0),
                corner(0.0, 64.0),
            ];
            let mut worst: f64 = 0.0;
            for y in 0..160 {
                for x in 0..200 {
                    let want = pixel_coverage(&poly, f64::from(x), f64::from(y));
                    let a = f64::from(rgba[(y as usize * 200 + x as usize) * 4 + 3]);
                    worst = worst.max((a - want).abs());
                }
            }
            eprintln!("rotation {rotation} scale {scale} {sampling:?}: coverage error {worst:.2e}");
            assert!(worst < 1e-4, "rotation {rotation} {sampling:?}: {worst}");
        }
    }
}

#[test]
fn level2_regions_and_determinism() {
    let dir = scratch("level2_regions_and_determinism");
    let path = dir.join("zone2.png");
    write_rgb(&path, SOURCE, SOURCE, |x, y| {
        zone(f64::from(x) + 0.5, f64::from(y) + 0.5)
    });
    let mut renderer = Renderer::headless().unwrap();
    let (doc, resolver) = image_doc(&path, 2, 0.71, 23.0);
    for sampling in [Sampling::Final, Sampling::Playback] {
        let mut request = RenderRequest::sized(Size::new(COMP, COMP), Precision::Float16);
        request.sampling = sampling;
        let full = render(&mut renderer, &doc, &resolver, &request);
        assert_eq!(
            full,
            render(&mut renderer, &doc, &resolver, &request),
            "determinism"
        );
        let region = PixelRect {
            x: 61,
            y: 90,
            width: 200,
            height: 150,
        };
        request.region = Some(region);
        let crop = render(&mut renderer, &doc, &resolver, &request);
        if std::env::var("PM_DEBUG_REGION").is_ok() {
            for (dx, dy) in [(0i64, 0i64), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                let mut diff = 0.0f32;
                for y in 2..148i64 {
                    for x in 2..198i64 {
                        let i = (((y + 90 + dy) * COMP as i64 + x + 61 + dx) * 4) as usize;
                        let j = ((y * 200 + x) * 4) as usize;
                        diff = diff.max((crop[j] - full[i]).abs());
                    }
                }
                eprintln!("{sampling:?} shift {dx},{dy}: max {diff}");
            }
            continue;
        }
        for y in 0..150usize {
            for x in 0..200usize {
                let i = ((y + 90) * COMP as usize + x + 61) * 4;
                assert_eq!(
                    &crop[(y * 200 + x) * 4..(y * 200 + x) * 4 + 4],
                    &full[i..i + 4],
                    "{sampling:?} region pixel {x},{y}"
                );
            }
        }
    }
}

/// A precomp of an edge (left half white) scaled up 4×.
fn precomp_doc(compat: u32) -> pm_model::Document {
    let edge = layer(
        "e",
        "solid",
        json!({ "color": "#FFFFFF", "w": 20.0, "h": 40.0 }),
        0.0,
        0.0,
        4.0,
    );
    let mut pc = layer(
        "pc",
        "precomp",
        json!({ "comp": "C1", "w": 40, "h": 40 }),
        10.0,
        10.0,
        4.0,
    );
    set(&mut pc, "scale.x", json!(400));
    set(&mut pc, "scale.y", json!(400));
    set(&mut pc, "rotation", json!(3));
    pc["compat"] = json!(compat);
    let mut doc: Value = project(200, 200, 4.0, no_fill(), vec![pc]);
    composition(&mut doc, "C1", 40, 40, 4.0, vec![edge]);
    doc["comps"]["C1"]["backgroundFill"] = no_fill();
    load(&doc)
}

#[test]
fn dynamic_precomp_mips_follow_the_requested_time() {
    let mut fill = layer(
        "fill",
        "solid",
        json!({ "color": "#FF0000", "w": 64, "h": 64 }),
        0.0,
        0.0,
        4.0,
    );
    set(
        &mut fill,
        "opacity",
        keys(json!(100), &[(0.0, json!(100)), (1.0, json!(25))]),
    );
    let mut pc = layer(
        "pc",
        "precomp",
        json!({ "comp": "C1", "w": 64, "h": 64 }),
        16.0,
        16.0,
        4.0,
    );
    set(&mut pc, "scale.x", json!(25));
    set(&mut pc, "scale.y", json!(25));
    let mut document = project(32, 32, 4.0, no_fill(), vec![pc]);
    composition(&mut document, "C1", 64, 64, 4.0, vec![fill]);
    document["comps"]["C1"]["backgroundFill"] = no_fill();
    let document = load(&document);
    let assets = MapResolver::default();
    let mut request = RenderRequest::sized(Size::new(32, 32), Precision::Float32);
    request.sampling = Sampling::Playback;
    let draw = |renderer: &mut Renderer, time| {
        let frame = renderer
            .render(&document, "Ptest-root", ticks(time), &request, &assets)
            .unwrap();
        assert!(frame.failures.is_empty());
        renderer.read_rgba(&frame).unwrap()
    };
    let mut warmed = Renderer::headless().unwrap();
    let first = draw(&mut warmed, 0.0);
    let changed = draw(&mut warmed, 1.0);
    let center = ((16 * 32 + 16) * 4) as usize;
    assert!((first[center + 3] - 1.0).abs() < 1e-6);
    assert!((changed[center + 3] - 0.25).abs() < 1e-6);
    assert_eq!(first, draw(&mut warmed, 0.0));
    assert_eq!(changed, draw(&mut Renderer::headless().unwrap(), 1.0));
}

#[test]
fn level2_precomps_rasterize_at_their_drawn_density() {
    let mut renderer = Renderer::headless().unwrap();
    let request = RenderRequest::sized(Size::new(200, 200), Precision::Float32);
    let empty = MapResolver::default();
    let inner = |doc: &pm_model::Document, renderer: &mut Renderer| {
        let (plan, _) = renderer
            .plan(doc, "Ptest-root", ticks(0.0), &request, &empty)
            .unwrap();
        let content = &plan.draws[0].content;
        let texture = match content {
            pm_render::plan::Content::Placed(p) => &p.content,
            other => other,
        };
        match texture {
            pm_render::plan::Content::Texture {
                source: pm_render::plan::Source::Comp(c),
                ..
            } => (c.width, c.height),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        inner(&precomp_doc(1), &mut renderer),
        (40, 40),
        "level 1: own size"
    );
    let (w, h) = inner(&precomp_doc(2), &mut renderer);
    assert_eq!((w, h), (160, 160), "level 2: four times the size");
    // The edge stays sharp: across a row, level 2 has at most 2 partial
    // pixels at the edge; level 1's bilinear magnification spreads it.
    let partial = |doc: &pm_model::Document, renderer: &mut Renderer| {
        let f = renderer
            .render(doc, "Ptest-root", ticks(0.0), &request, &empty)
            .unwrap();
        let rgba = renderer.read_rgba(&f).unwrap();
        let y = 90usize;
        (40..120usize)
            .filter(|x| {
                let a = rgba[(y * 200 + x) * 4 + 3];
                a > 0.02 && a < 0.98
            })
            .count()
    };
    let p1 = partial(&precomp_doc(1), &mut renderer);
    let p2 = partial(&precomp_doc(2), &mut renderer);
    eprintln!("partial edge pixels: level 1 {p1}, level 2 {p2}");
    assert!(p2 <= 2 && p1 > p2, "level 1 {p1} level 2 {p2}");
}

#[test]
fn level2_content_of_every_layer_type_is_drawn() {
    use pm_render::{Feature, unsupported};
    let mk = |ty: &str, d: Value, format: Option<&str>| {
        let mut l = layer("l", ty, d, 0.0, 0.0, 2.0);
        l["compat"] = json!(2);
        let mut doc = project(100, 100, 2.0, no_fill(), vec![l]);
        if let Some(f) = format {
            asset(&mut doc, "a", "image", f, 10, 10, 0.0);
        }
        composition(&mut doc, "C1", 10, 10, 2.0, vec![]);
        unsupported(&load(&doc), "Ptest-root")
            .into_iter()
            .map(|u| u.feature)
            .collect::<Vec<_>>()
    };
    let level2 = |f: &[Feature]| f.contains(&Feature::CompatLevel2);
    assert!(!level2(&mk("image", json!({ "asset": "a" }), Some("png"))));
    assert!(!level2(&mk("video", json!({ "asset": "a" }), None)));
    assert!(!level2(&mk("precomp", json!({ "comp": "C1" }), None)));
    // Level 2 SVG images and solids are drawn by `crate::vector`.
    assert!(!level2(&mk("image", json!({ "asset": "a" }), Some("svg"))));
    assert!(!level2(&mk("solid", json!({ "color": "#FFFFFF" }), None)));
    // Text and captions at output resolution, shader layers resampled, and
    // layers without content of their own.
    for (ty, d) in [
        ("text", json!({ "text": "Level 2" })),
        ("captions", json!({})),
        ("shader", json!({ "code": "void main(){ o = vec4(1.0); }" })),
        ("group", json!({})),
        ("null", json!({})),
        ("audio", json!({})),
        ("adjustment", json!({})),
    ] {
        assert!(!level2(&mk(ty, d, None)), "{ty}");
    }
    // Typography, selectors and anchoring are native level 2 semantics.
    // This fontless capability check may still name its missing fonts;
    // it must never resurrect the old level 2 text routing reason.
    for d in [
        json!({ "text": "x", "animators": [{ "id": "a" }] }),
        json!({ "text": "x", "styles": [{ "start": 0, "end": 1, "p": { "size": 32 } }] }),
        json!({ "text": "x", "fontAnchorBounds": { "x0": 0, "y0": 0, "x1": 20, "y1": 30 } }),
    ] {
        let features = mk("text", d, None);
        assert!(
            !features.iter().any(|f| matches!(f, Feature::CompatLevel2)
                || matches!(f, Feature::Text(t) if t.contains("compat level 2"))),
            "{features:?}"
        );
    }
}
