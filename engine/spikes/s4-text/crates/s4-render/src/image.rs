//! Float RGBA images (premultiplied, top-down) and the comparison metrics.

use std::io::BufWriter;
use std::path::Path;

#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA, row-major from the top.
    pub data: Vec<f32>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Self {
        Image { width, height, data: vec![0.0; width as usize * height as usize * 4] }
    }

    /// From 8-bit RGBA; `premultiplied` says whether the bytes already are.
    pub fn from_rgba8(width: u32, height: u32, bytes: &[u8], premultiplied: bool) -> Self {
        let mut data = Vec::with_capacity(bytes.len());
        for px in bytes.chunks_exact(4) {
            let a = px[3] as f32 / 255.0;
            for c in &px[..3] {
                let v = *c as f32 / 255.0;
                data.push(if premultiplied { v } else { v * a });
            }
            data.push(a);
        }
        Image { width, height, data }
    }

    pub fn channel(&self, c: usize) -> Vec<f32> {
        self.data.iter().skip(c).step_by(4).copied().collect()
    }

    /// Writes an 8-bit straight-alpha PNG composited over `background` for
    /// review (`None` keeps transparency).
    pub fn write_png(&self, path: &Path, background: Option<[f32; 3]>) {
        let file = std::fs::File::create(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut enc = png::Encoder::new(BufWriter::new(file), self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        let mut out = Vec::with_capacity(self.data.len());
        for px in self.data.chunks_exact(4) {
            let a = px[3].clamp(0.0, 1.0);
            match background {
                Some(bg) => {
                    for c in 0..3 {
                        out.push(((px[c] + bg[c] * (1.0 - a)).clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                    }
                    out.push(255);
                }
                None => {
                    for c in 0..3 {
                        let v = if a > 0.0 { px[c] / a } else { 0.0 };
                        out.push((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                    }
                    out.push((a * 255.0 + 0.5) as u8);
                }
            }
        }
        w.write_image_data(&out).unwrap();
    }

    /// Stacks images vertically (a review sheet: one row per renderer).
    pub fn stack(images: &[&Image]) -> Image {
        let width = images.iter().map(|i| i.width).max().unwrap_or(1);
        let height = images.iter().map(|i| i.height + 2).sum::<u32>();
        let mut out = Image::new(width, height);
        let mut y0 = 0usize;
        for img in images {
            for y in 0..img.height as usize {
                let src = &img.data[y * img.width as usize * 4..(y + 1) * img.width as usize * 4];
                let dst = (y0 + y) * width as usize * 4;
                out.data[dst..dst + src.len()].copy_from_slice(src);
            }
            // A one-pixel grey separator.
            let sep = (y0 + img.height as usize) * width as usize * 4;
            for x in 0..width as usize {
                out.data[sep + x * 4..sep + x * 4 + 4].copy_from_slice(&[0.25, 0.25, 0.25, 1.0]);
            }
            y0 += img.height as usize + 2;
        }
        out
    }

    /// Applies `alpha' = alpha^gamma` to coverage, scaling the premultiplied
    /// color with it (a coverage curve in a coverage-then-paint pipeline).
    pub fn with_coverage_gamma(&self, gamma: f32) -> Image {
        let mut out = self.clone();
        for px in out.data.chunks_exact_mut(4) {
            let a = px[3];
            if a > 0.0 {
                let a2 = a.clamp(0.0, 1.0).powf(gamma);
                let k = a2 / a;
                px[0] *= k;
                px[1] *= k;
                px[2] *= k;
                px[3] = a2;
            }
        }
        out
    }
}

/// Separable Gaussian blur (σ = 1.5, 11 taps), "valid" region only.
fn gauss(src: &[f32], w: usize, h: usize) -> (Vec<f32>, usize, usize) {
    const R: usize = 5;
    let k: Vec<f32> = {
        let raw: Vec<f32> = (0..=2 * R).map(|i| (-((i as f32 - R as f32).powi(2)) / (2.0 * 1.5 * 1.5)).exp()).collect();
        let s: f32 = raw.iter().sum();
        raw.iter().map(|v| v / s).collect()
    };
    if w <= 2 * R || h <= 2 * R {
        return (Vec::new(), 0, 0);
    }
    let (vw, vh) = (w - 2 * R, h - 2 * R);
    let mut tmp = vec![0f32; vw * h];
    for y in 0..h {
        for x in 0..vw {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                acc += kv * src[y * w + x + i];
            }
            tmp[y * vw + x] = acc;
        }
    }
    let mut out = vec![0f32; vw * vh];
    for y in 0..vh {
        for x in 0..vw {
            let mut acc = 0.0;
            for (i, kv) in k.iter().enumerate() {
                acc += kv * tmp[(y + i) * vw + x];
            }
            out[y * vw + x] = acc;
        }
    }
    (out, vw, vh)
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Ssim {
    /// Mean SSIM over the whole image (valid region).
    pub full: f64,
    /// Mean SSIM over windows that contain ink in either image (local mean
    /// coverage above 1/512): empty background does not dilute it.
    pub ink: f64,
}

/// SSIM (Wang et al. 2004: 11×11 Gaussian, σ 1.5, K1 0.01, K2 0.03, L 1).
pub fn ssim(a: &[f32], b: &[f32], w: usize, h: usize) -> Ssim {
    let sq = |v: &[f32]| v.iter().map(|x| x * x).collect::<Vec<_>>();
    let ab: Vec<f32> = a.iter().zip(b).map(|(x, y)| x * y).collect();
    let (ma, vw, vh) = gauss(a, w, h);
    if vw == 0 {
        return Ssim { full: 1.0, ink: 1.0 };
    }
    let (mb, _, _) = gauss(b, w, h);
    let (saa, _, _) = gauss(&sq(a), w, h);
    let (sbb, _, _) = gauss(&sq(b), w, h);
    let (sab, _, _) = gauss(&ab, w, h);
    let (c1, c2) = (0.01f64 * 0.01, 0.03f64 * 0.03);
    let (mut full, mut ink, mut n_ink) = (0.0f64, 0.0f64, 0usize);
    for i in 0..vw * vh {
        let (mx, my) = (ma[i] as f64, mb[i] as f64);
        let vx = (saa[i] as f64 - mx * mx).max(0.0);
        let vy = (sbb[i] as f64 - my * my).max(0.0);
        let cxy = sab[i] as f64 - mx * my;
        let s = ((2.0 * mx * my + c1) * (2.0 * cxy + c2)) / ((mx * mx + my * my + c1) * (vx + vy + c2));
        full += s;
        if mx > 1.0 / 512.0 || my > 1.0 / 512.0 {
            ink += s;
            n_ink += 1;
        }
    }
    Ssim { full: full / (vw * vh) as f64, ink: if n_ink > 0 { ink / n_ink as f64 } else { 1.0 } }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compare {
    /// SSIM of the alpha (coverage) channel.
    pub alpha: Ssim,
    /// The lowest SSIM over the premultiplied R, G, B, A channels.
    pub min_channel: Ssim,
    /// Mean |Δalpha| over pixels with coverage in either image.
    pub alpha_mae_ink: f64,
    pub alpha_max_abs: f64,
    /// Σ alpha(test) / Σ alpha(reference): above 1 is heavier (darker) text.
    pub ink_ratio: f64,
}

pub fn compare(test: &Image, reference: &Image) -> Compare {
    assert_eq!((test.width, test.height), (reference.width, reference.height));
    let (w, h) = (test.width as usize, test.height as usize);
    let mut channels = Vec::new();
    for c in 0..4 {
        channels.push(ssim(&test.channel(c), &reference.channel(c), w, h));
    }
    let alpha = channels[3];
    let min_channel = Ssim {
        full: channels.iter().map(|s| s.full).fold(f64::INFINITY, f64::min),
        ink: channels.iter().map(|s| s.ink).fold(f64::INFINITY, f64::min),
    };
    let (ta, ra) = (test.channel(3), reference.channel(3));
    let (mut mae, mut n, mut max, mut st, mut sr) = (0.0f64, 0usize, 0.0f64, 0.0f64, 0.0f64);
    for (x, y) in ta.iter().zip(&ra) {
        let (x, y) = (*x as f64, *y as f64);
        st += x;
        sr += y;
        let d = (x - y).abs();
        max = max.max(d);
        if x > 0.0 || y > 0.0 {
            mae += d;
            n += 1;
        }
    }
    Compare { alpha, min_channel, alpha_mae_ink: if n > 0 { mae / n as f64 } else { 0.0 }, alpha_max_abs: max, ink_ratio: if sr > 0.0 { st / sr } else { 1.0 } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_have_unit_ssim() {
        let w = 40;
        let a: Vec<f32> = (0..w * w).map(|i| ((i * 37) % 101) as f32 / 100.0).collect();
        let s = ssim(&a, &a, w, w);
        assert!((s.full - 1.0).abs() < 1e-9 && (s.ink - 1.0).abs() < 1e-9);
    }

    #[test]
    fn shifted_edge_lowers_ssim() {
        let w = 40;
        let a: Vec<f32> = (0..w * w).map(|i| if i % w < 20 { 1.0 } else { 0.0 }).collect();
        let b: Vec<f32> = (0..w * w).map(|i| if i % w < 21 { 1.0 } else { 0.0 }).collect();
        let s = ssim(&a, &b, w, w);
        assert!(s.ink < 0.95 && s.ink > 0.3, "{s:?}");
    }

    #[test]
    fn gamma_one_is_identity() {
        let mut img = Image::new(2, 1);
        img.data.copy_from_slice(&[0.5, 0.5, 0.5, 0.5, 0.2, 0.1, 0.0, 0.25]);
        assert_eq!(img.with_coverage_gamma(1.0).data, img.data);
    }
}
