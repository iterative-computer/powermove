//! 8-bit RGBA frames, CIELAB (D65, sRGB primaries) and CIEDE2000, the
//! perceptual distance the frame graders use.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    /// Row-major, 4 bytes per pixel, straight alpha.
    pub data: Vec<u8>,
}

impl Rgba {
    pub fn solid(width: u32, height: u32, rgb: [u8; 3]) -> Self {
        let px = [rgb[0], rgb[1], rgb[2], 255];
        Self {
            width,
            height,
            data: px.repeat((width * height) as usize),
        }
    }
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.data[i..i + 4].try_into().unwrap()
    }
    pub fn set(&mut self, x: u32, y: u32, px: [u8; 4]) {
        let i = ((y * self.width + x) * 4) as usize;
        self.data[i..i + 4].copy_from_slice(&px);
    }

    pub fn decode_png(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
            .read_info()
            .map_err(|e| format!("PNG: {e}"))?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("PNG: {e}"))?;
        buf.truncate(info.buffer_size());
        if info.bit_depth != png::BitDepth::Eight {
            return Err("PNG is not 8-bit".into());
        }
        let data = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => buf
                .chunks_exact(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::GrayscaleAlpha => buf
                .chunks_exact(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Indexed => return Err("indexed PNGs are not supported".into()),
        };
        Ok(Self {
            width: info.width,
            height: info.height,
            data,
        })
    }

    pub fn encode_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("PNG header");
        w.write_image_data(&self.data).expect("PNG data");
        drop(w);
        out
    }

    /// Pixels inside a normalised rectangle, composited over black (frames are
    /// opaque renders; alpha only matters for synthetic inputs).
    pub fn region(&self, r: &Rect) -> Vec<[u8; 3]> {
        let (x0, y0, x1, y1) = r.pixels(self.width, self.height);
        let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = self.pixel(x, y);
                let a = p[3] as u32;
                out.push([
                    (p[0] as u32 * a / 255) as u8,
                    (p[1] as u32 * a / 255) as u8,
                    (p[2] as u32 * a / 255) as u8,
                ]);
            }
        }
        out
    }
}

/// A rectangle in normalised frame coordinates (0..1, origin top left).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const FULL: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    };
    pub fn pixels(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let cx = |v: f64| (v.clamp(0.0, 1.0) * width as f64).round() as u32;
        let cy = |v: f64| (v.clamp(0.0, 1.0) * height as f64).round() as u32;
        let (x0, y0) = (cx(self.x), cy(self.y));
        let (x1, y1) = (
            cx(self.x + self.w).max(x0 + 1),
            cy(self.y + self.h).max(y0 + 1),
        );
        (x0, y0, x1.min(width), y1.min(height))
    }
}

fn linear(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB (D65) to CIELAB (D65 white).
pub fn lab(rgb: [u8; 3]) -> [f64; 3] {
    let (r, g, b) = (linear(rgb[0]), linear(rgb[1]), linear(rgb[2]));
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047;
    let y = 0.2126729 * r + 0.7151522 * g + 0.0721750 * b;
    let z = (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883;
    let f = |t: f64| {
        if t > 216.0 / 24389.0 {
            t.cbrt()
        } else {
            (24389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 colour difference (kL = kC = kH = 1).
pub fn delta_e2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let (l1, a1, b1) = (a[0], a[1], a[2]);
    let (l2, a2, b2) = (b[0], b[1], b[2]);
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let cbar = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cbar.powi(7) / (cbar.powi(7) + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = ((a1p * a1p + b1 * b1).sqrt(), (a2p * a2p + b2 * b2).sqrt());
    let hue = |b: f64, a: f64| {
        if b == 0.0 && a == 0.0 {
            0.0
        } else {
            let h = b.atan2(a).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else if (h2p - h1p).abs() <= 180.0 {
        h2p - h1p
    } else if h2p - h1p > 180.0 {
        h2p - h1p - 360.0
    } else {
        h2p - h1p + 360.0
    };
    let dhp_big = 2.0 * (c1p * c2p).sqrt() * (dhp.to_radians() / 2.0).sin();
    let lbarp = (l1 + l2) / 2.0;
    let cbarp = (c1p + c2p) / 2.0;
    let hbarp = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= 180.0 {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 360.0 {
        (h1p + h2p + 360.0) / 2.0
    } else {
        (h1p + h2p - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * (hbarp - 30.0).to_radians().cos()
        + 0.24 * (2.0 * hbarp).to_radians().cos()
        + 0.32 * (3.0 * hbarp + 6.0).to_radians().cos()
        - 0.20 * (4.0 * hbarp - 63.0).to_radians().cos();
    let dtheta = 30.0 * (-((hbarp - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cbarp.powi(7) / (cbarp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lbarp - 50.0).powi(2) / (20.0 + (lbarp - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cbarp;
    let sh = 1.0 + 0.015 * cbarp * t;
    let rt = -(2.0 * dtheta * PI / 180.0).sin() * rc;
    ((dlp / sl).powi(2)
        + (dcp / sc).powi(2)
        + (dhp_big / sh).powi(2)
        + rt * (dcp / sc) * (dhp_big / sh))
        .sqrt()
}

/// Per-pixel CIEDE2000 between two equally sized frames over a region.
pub fn delta_e_map(a: &Rgba, b: &Rgba, r: &Rect) -> Result<Vec<f64>, String> {
    if (a.width, a.height) != (b.width, b.height) {
        return Err(format!(
            "frame sizes differ: {}x{} vs {}x{}",
            a.width, a.height, b.width, b.height
        ));
    }
    Ok(a.region(r)
        .into_iter()
        .zip(b.region(r))
        .map(|(p, q)| delta_e2000(lab(p), lab(q)))
        .collect())
}

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// Nearest-rank percentile, `p` in 0..=100.
pub fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * s.len() as f64).ceil().max(1.0) as usize;
    s[rank.min(s.len()) - 1]
}

/// Parse `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(…)`/`rgba(…)`.
pub fn parse_css_color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let n = |i: usize, w: usize| u8::from_str_radix(&hex[i..i + w], 16).ok();
        return match hex.len() {
            3 | 4 => Some([n(0, 1)? * 17, n(1, 1)? * 17, n(2, 1)? * 17]),
            6 | 8 => Some([n(0, 2)?, n(2, 2)?, n(4, 2)?]),
            _ => None,
        };
    }
    let inner = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let parts: Vec<&str> = inner
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    let mut out = [0u8; 3];
    for (o, p) in out.iter_mut().zip(&parts[..3]) {
        *o = if let Some(pc) = p.strip_suffix('%') {
            (pc.parse::<f64>().ok()?.clamp(0.0, 100.0) * 2.55).round() as u8
        } else {
            p.parse::<f64>().ok()?.clamp(0.0, 255.0).round() as u8
        };
    }
    Some(out)
}

/// WCAG 2 relative luminance and contrast ratio.
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let lum = |c: [u8; 3]| 0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2]);
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// CIELAB hue angle in degrees (0..360) and chroma.
pub fn hue_chroma(c: [u8; 3]) -> (f64, f64) {
    let l = lab(c);
    let h = l[2].atan2(l[1]).to_degrees();
    (
        if h < 0.0 { h + 360.0 } else { h },
        (l[1] * l[1] + l[2] * l[2]).sqrt(),
    )
}
