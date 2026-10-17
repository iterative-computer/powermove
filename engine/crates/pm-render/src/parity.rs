//! Section 2's 3D bar as a reusable score (the parity lane's `pm-cli parity`
//! and this crate's tests): at most 2 CV for 99.5% of pixels, at most 12 CV
//! for any pixel outside MSAA edges (and shadow penumbrae, none in Phase 2),
//! and per-object SSIM at least 0.99.
//!
//! Definitions follow the S7 spike's scorer (`docs/engine/spikes/S7.md`):
//! differences are per channel on 8-bit premultiplied RGBA and a pixel's
//! difference is its largest channel difference; MSAA edges are the pixels
//! where the reference rendered without MSAA differs from the reference with
//! MSAA, dilated by one pixel; SSIM is the mean of the four per-channel SSIM
//! maps (11×11 Gaussian, σ = 1.5, K1 = 0.01, K2 = 0.03, L = 255) over an
//! object's pixels.

pub const MAX_FRACTION_CV: u8 = 2;
pub const FRACTION_TARGET: f64 = 0.995;
pub const MAX_CV_OUTSIDE: u8 = 12;
pub const SSIM_TARGET: f64 = 0.99;

/// One output's score against the 3D bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Bar3d {
    pub pixels: usize,
    pub fraction_within_2: f64,
    pub max_cv: u8,
    pub max_cv_outside_edges: u8,
    /// Pixels excluded from the 12 CV maximum (MSAA edges).
    pub excluded: usize,
    /// Pixels over 12 CV outside the excluded set.
    pub over_12_outside: usize,
    /// SSIM over the object's pixels (`None` without object pixels).
    pub object_ssim: Option<f64>,
}

impl Bar3d {
    pub fn pass_fraction(&self) -> bool {
        self.fraction_within_2 >= FRACTION_TARGET
    }

    pub fn pass_max(&self) -> bool {
        self.max_cv_outside_edges <= MAX_CV_OUTSIDE
    }

    pub fn pass_ssim(&self) -> bool {
        self.object_ssim.is_none_or(|s| s >= SSIM_TARGET)
    }

    pub fn pass(&self) -> bool {
        self.pass_fraction() && self.pass_max() && self.pass_ssim()
    }
}

/// `mask` grown by one pixel in every direction.
pub fn dilate(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = mask.to_vec();
    for y in 0..h {
        for x in 0..w {
            if !mask[y * w + x] {
                continue;
            }
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        out[ny as usize * w + nx as usize] = true;
                    }
                }
            }
        }
    }
    out
}

/// Pixels whose value depends on multisampling: `with` and `without` MSAA
/// differ, dilated by one.
pub fn msaa_edges(with: &[u8], without: &[u8], w: usize, h: usize) -> Vec<bool> {
    let m: Vec<bool> = (0..w * h)
        .map(|i| (0..4).any(|c| with[i * 4 + c] != without[i * 4 + c]))
        .collect();
    dilate(&m, w, h)
}

fn gaussian_blur(src: &[f64], w: usize, h: usize) -> Vec<f64> {
    let sigma = 1.5f64;
    let k: Vec<f64> = (-5i32..=5)
        .map(|i| libm::exp(-f64::from(i * i) / (2.0 * sigma * sigma)))
        .collect();
    let sum: f64 = k.iter().sum();
    let k: Vec<f64> = k.iter().map(|v| v / sum).collect();
    let mut tmp = vec![0.0; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let sx = (x as i64 + i as i64 - 5).clamp(0, w as i64 - 1) as usize;
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
                let sy = (y as i64 + i as i64 - 5).clamp(0, h as i64 - 1) as usize;
                acc += kv * tmp[sy * w + x];
            }
            out[y * w + x] = acc;
        }
    }
    out
}

/// The mean of the four per-channel SSIM maps of two RGBA8 images.
pub fn ssim_map(a: &[u8], b: &[u8], w: usize, h: usize) -> Vec<f64> {
    let c1 = (0.01f64 * 255.0).powi(2);
    let c2 = (0.03f64 * 255.0).powi(2);
    let mut total = vec![0.0; w * h];
    for ch in 0..4 {
        let x: Vec<f64> = (0..w * h).map(|i| f64::from(a[i * 4 + ch])).collect();
        let y: Vec<f64> = (0..w * h).map(|i| f64::from(b[i * 4 + ch])).collect();
        let mx = gaussian_blur(&x, w, h);
        let my = gaussian_blur(&y, w, h);
        let xx = gaussian_blur(&x.iter().map(|v| v * v).collect::<Vec<_>>(), w, h);
        let yy = gaussian_blur(&y.iter().map(|v| v * v).collect::<Vec<_>>(), w, h);
        let xy = gaussian_blur(
            &x.iter().zip(&y).map(|(p, q)| p * q).collect::<Vec<_>>(),
            w,
            h,
        );
        for i in 0..w * h {
            let (sx, sy, sxy) = (
                xx[i] - mx[i] * mx[i],
                yy[i] - my[i] * my[i],
                xy[i] - mx[i] * my[i],
            );
            let s = ((2.0 * mx[i] * my[i] + c1) * (2.0 * sxy + c2))
                / ((mx[i] * mx[i] + my[i] * my[i] + c1) * (sx + sy + c2));
            total[i] += s / 4.0;
        }
    }
    total
}

/// Section 25's text class score of `test` against `reference`
/// (premultiplied RGBA8, `w` × `h`): `(ink SSIM, full SSIM, ink ratio)`.
/// Ink SSIM is the mean of the SSIM map over ink windows, the pixels whose
/// local mean alpha (the SSIM window) exceeds 1/512 in either image, so
/// empty background cannot inflate it (spike S4's strict score). The ink
/// ratio is Σα(test) / Σα(reference). Only the ink's bounding box (grown by
/// the window) is evaluated; outside it both images are empty and SSIM is 1.
pub fn text_ssim(reference: &[u8], test: &[u8], w: usize, h: usize) -> (f64, f64, f64) {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    let (mut ra, mut ta) = (0.0f64, 0.0f64);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let (a, b) = (reference[i + 3], test[i + 3]);
            ra += f64::from(a);
            ta += f64::from(b);
            if a > 0 || b > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    let ratio = if ra > 0.0 {
        ta / ra
    } else if ta > 0.0 {
        f64::INFINITY
    } else {
        1.0
    };
    if x1 <= x0 {
        return (1.0, 1.0, ratio);
    }
    let (bx0, by0) = (x0.saturating_sub(6), y0.saturating_sub(6));
    let (bx1, by1) = ((x1 + 6).min(w), (y1 + 6).min(h));
    let (bw, bh) = (bx1 - bx0, by1 - by0);
    let crop = |img: &[u8]| -> Vec<u8> {
        let mut out = Vec::with_capacity(bw * bh * 4);
        for y in by0..by1 {
            out.extend_from_slice(&img[(y * w + bx0) * 4..(y * w + bx1) * 4]);
        }
        out
    };
    let (cr, ct) = (crop(reference), crop(test));
    let map = ssim_map(&cr, &ct, bw, bh);
    let alpha = |img: &[u8]| -> Vec<f64> {
        img.chunks_exact(4)
            .map(|p| f64::from(p[3]) / 255.0)
            .collect()
    };
    let (mr, mt) = (
        gaussian_blur(&alpha(&cr), bw, bh),
        gaussian_blur(&alpha(&ct), bw, bh),
    );
    let (mut sum, mut n) = (0.0, 0usize);
    for i in 0..bw * bh {
        if mr[i] > 1.0 / 512.0 || mt[i] > 1.0 / 512.0 {
            sum += map[i];
            n += 1;
        }
    }
    let total: f64 = map.iter().sum::<f64>() + (w * h - bw * bh) as f64;
    let full = total / (w * h) as f64;
    let ink = if n == 0 { full } else { sum / n as f64 };
    (ink, full, ratio)
}

/// Scores `test` against `reference` (both RGBA8, `w` × `h`). `excluded`
/// marks MSAA edges; `object` marks the object's pixels for SSIM.
pub fn bar_3d(
    reference: &[u8],
    test: &[u8],
    excluded: &[bool],
    object: &[bool],
    w: usize,
    h: usize,
) -> Bar3d {
    let n = w * h;
    let mut within = 0;
    let mut max_cv = 0u8;
    let mut max_outside = 0u8;
    let mut over = 0;
    for i in 0..n {
        let d = (0..4)
            .map(|c| reference[i * 4 + c].abs_diff(test[i * 4 + c]))
            .max()
            .unwrap_or(0);
        if d <= MAX_FRACTION_CV {
            within += 1;
        }
        max_cv = max_cv.max(d);
        if !excluded[i] {
            max_outside = max_outside.max(d);
            if d > MAX_CV_OUTSIDE {
                over += 1;
            }
        }
    }
    let ssim = ssim_map(reference, test, w, h);
    let (mut sum, mut count) = (0.0, 0usize);
    for i in 0..n {
        if object[i] {
            sum += ssim[i];
            count += 1;
        }
    }
    Bar3d {
        pixels: n,
        fraction_within_2: within as f64 / n.max(1) as f64,
        max_cv,
        max_cv_outside_edges: max_outside,
        excluded: excluded.iter().filter(|e| **e).count(),
        over_12_outside: over,
        object_ssim: (count > 0).then(|| sum / count as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_pass() {
        let img: Vec<u8> = (0..16 * 16 * 4).map(|i| (i * 7 % 256) as u8).collect();
        let none = vec![false; 256];
        let all = vec![true; 256];
        let s = bar_3d(&img, &img, &none, &all, 16, 16);
        assert!(s.pass());
        assert_eq!(s.max_cv, 0);
        assert!((s.object_ssim.unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_large_difference_outside_edges_fails() {
        let a = vec![100u8; 8 * 8 * 4];
        let mut b = a.clone();
        b[0] = 120;
        let none = vec![false; 64];
        let s = bar_3d(&a, &b, &none, &none, 8, 8);
        assert!(!s.pass_max());
        let mut edge = none.clone();
        edge[0] = true;
        assert!(bar_3d(&a, &b, &edge, &none, 8, 8).pass_max());
    }

    #[test]
    fn dilation_reaches_neighbours() {
        let mut m = vec![false; 9];
        m[4] = true;
        assert!(dilate(&m, 3, 3).iter().all(|v| *v));
    }
}
