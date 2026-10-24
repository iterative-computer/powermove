//! Colour maths written from the standards, for reference renders that must
//! not share code with the engine (tests/video_reading.rs). Nothing here
//! calls pm-color, pm-media's normalization or the probe.

/// ITU-T H.273 quantization of an n-bit code (equations 20 to 31): limited
/// (narrow) range scales Y′ by 219·2ⁿ⁻⁸ above 16·2ⁿ⁻⁸ and chroma by
/// 224·2ⁿ⁻⁸ about 2ⁿ⁻¹; full range by 2ⁿ − 1 about 2ⁿ⁻¹.
pub fn normalize(code: f64, bits: u32, full: bool, chroma: bool) -> f64 {
    let k = f64::from(1u32 << (bits - 8));
    let max = f64::from((1u64 << bits) as u32 - 1);
    let mid = f64::from(1u32 << (bits - 1));
    match (full, chroma) {
        (false, false) => (code - 16.0 * k) / (219.0 * k),
        (false, true) => (code - 128.0 * k) / (224.0 * k),
        (true, false) => code / max,
        (true, true) => (code - mid) / max,
    }
}

/// Kr, Kb of H.273 matrix coefficients 1 (BT.709), 5/6 (BT.601), 9 (BT.2020
/// non-constant luminance).
pub fn kr_kb(matrix: &str) -> (f64, f64) {
    match matrix {
        "bt709" => (0.2126, 0.0722),
        "bt601" => (0.299, 0.114),
        "bt2020" => (0.2627, 0.0593),
        other => panic!("matrix {other}"),
    }
}

/// Y′CbCr (Y′ 0..1, C −0.5..0.5) to R′G′B′ (H.273 equations 38 to 40).
pub fn ycbcr_to_rgb(y: f64, cb: f64, cr: f64, (kr, kb): (f64, f64)) -> [f64; 3] {
    let r = y + 2.0 * (1.0 - kr) * cr;
    let b = y + 2.0 * (1.0 - kb) * cb;
    [r, (y - kr * r - kb * b) / (1.0 - kr - kb), b]
}

/// Published chromaticities (x, y) of R, G, B and the D65 white point:
/// BT.709, SMPTE 170M, BT.470 System B/G, BT.2020, SMPTE EG 432-1 (P3 D65).
pub fn chromaticities(primaries: &str) -> [[f64; 2]; 4] {
    const D65: [f64; 2] = [0.3127, 0.3290];
    match primaries {
        "bt709" => [[0.640, 0.330], [0.300, 0.600], [0.150, 0.060], D65],
        "smpte170m" => [[0.630, 0.340], [0.310, 0.595], [0.155, 0.070], D65],
        "bt470bg" => [[0.640, 0.330], [0.290, 0.600], [0.150, 0.060], D65],
        "bt2020" => [[0.708, 0.292], [0.170, 0.797], [0.131, 0.046], D65],
        "p3" => [[0.680, 0.320], [0.265, 0.690], [0.150, 0.060], D65],
        other => panic!("primaries {other}"),
    }
}

type M3 = [[f64; 3]; 3];

fn mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn inverse(m: &M3) -> M3 {
    let c = |i: usize, j: usize| {
        let (r0, r1) = ((i + 1) % 3, (i + 2) % 3);
        let (c0, c1) = ((j + 1) % 3, (j + 2) % 3);
        m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
    };
    let det: f64 = (0..3).map(|j| m[0][j] * c(0, j)).sum();
    std::array::from_fn(|i| std::array::from_fn(|j| c(j, i) / det))
}

/// RGB to XYZ (SMPTE RP 177's normalized primary matrix).
pub fn npm(primaries: &str) -> M3 {
    let c = chromaticities(primaries);
    let xyz = |[x, y]: [f64; 2]| [x / y, 1.0, (1.0 - x - y) / y];
    let p: M3 = std::array::from_fn(|i| std::array::from_fn(|j| xyz(c[j])[i]));
    let w = xyz(c[3]);
    let inv = inverse(&p);
    let s: [f64; 3] = std::array::from_fn(|i| (0..3).map(|k| inv[i][k] * w[k]).sum());
    std::array::from_fn(|i| std::array::from_fn(|j| p[i][j] * s[j]))
}

/// Linear-light RGB in `from` primaries to Rec.709.
pub fn to_709(from: &str) -> M3 {
    mul(&inverse(&npm("bt709")), &npm(from))
}

pub fn apply(m: &M3, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| (0..3).map(|k| m[i][k] * v[k]).sum())
}

/// IEC 61966-2-1 (sRGB) decode and encode, extended odd-symmetrically so
/// superwhites and sub-blacks are kept.
pub fn srgb_decode(e: f64) -> f64 {
    let m = e.abs();
    let l = if m <= 0.04045 {
        m / 12.92
    } else {
        ((m + 0.055) / 1.055).powf(2.4)
    };
    l.copysign(e)
}

pub fn srgb_encode(l: f64) -> f64 {
    let m = l.abs();
    let e = if m <= 0.0031308 {
        12.92 * m
    } else {
        1.055 * m.powf(1.0 / 2.4) - 0.055
    };
    e.copysign(l)
}

// SMPTE ST 2084.
const M1: f64 = 2610.0 / 16384.0;
const M2: f64 = 2523.0 / 4096.0 * 128.0;
const C1: f64 = 3424.0 / 4096.0;
const C2: f64 = 2413.0 / 4096.0 * 32.0;
const C3: f64 = 2392.0 / 4096.0 * 32.0;

/// ST 2084 EOTF: signal to cd/m².
pub fn pq_eotf(e: f64) -> f64 {
    let p = e.max(0.0).powf(1.0 / M2);
    10000.0 * ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1)
}

/// ST 2084 inverse EOTF: cd/m² to signal.
pub fn pq_signal(nits: f64) -> f64 {
    let y = (nits / 10000.0).max(0.0).powf(M1);
    ((C1 + C2 * y) / (1.0 + C3 * y)).powf(M2)
}

/// BT.2100 HLG: signal to display light in cd/m² on a 1000 cd/m² display
/// (inverse OETF, then the OOTF with γ = 1.2 and BT.2020 luminance).
pub fn hlg_display(rgb: [f64; 3]) -> [f64; 3] {
    let (a, b): (f64, f64) = (0.17883277, 1.0 - 4.0 * 0.17883277);
    let c = 0.5 - a * (4.0 * a).ln();
    let scene = rgb.map(|e| {
        let e = e.max(0.0);
        if e <= 0.5 {
            e * e / 3.0
        } else {
            (((e - c) / a).exp() + b) / 12.0
        }
    });
    let ys = 0.2627 * scene[0] + 0.6780 * scene[1] + 0.0593 * scene[2];
    let gain = if ys > 0.0 { ys.powf(1.2 - 1.0) } else { 0.0 };
    scene.map(|s| 1000.0 * gain * s)
}

/// BT.2390 EETF (section 5.4.1) on a PQ signal: source peak `lw`, target
/// peak `lt` (black 0, so no toe), as a PQ signal.
pub fn eetf(e: f64, lw: f64, lt: f64) -> f64 {
    let pw = pq_signal(lw);
    let max_lum = pq_signal(lt) / pw;
    let e1 = (e / pw).clamp(0.0, 1.0);
    let ks = 1.5 * max_lum - 0.5;
    let e2 = if e1 < ks {
        e1
    } else {
        let t = (e1 - ks) / (1.0 - ks);
        (2.0 * t.powi(3) - 3.0 * t.powi(2) + 1.0) * ks
            + (t.powi(3) - 2.0 * t.powi(2) + t) * (1.0 - ks)
            + (-2.0 * t.powi(3) + 3.0 * t.powi(2)) * max_lum
    };
    e2 * pw
}

/// Lanczos-3 weight.
fn lanczos3(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else if x.abs() >= 3.0 {
        0.0
    } else {
        let px = std::f64::consts::PI * x;
        3.0 * px.sin() * (px / 3.0).sin() / (px * px)
    }
}

/// One subsampled chroma plane reconstructed at luma resolution with a
/// separable Lanczos-3 filter at the chroma samples' positions (`origin`,
/// in luma pixels: where chroma sample 0 sits), normalized weights, edges
/// repeated.
pub fn reconstruct(
    plane: &[f64],
    (cw, ch): (usize, usize),
    (w, h): (usize, usize),
    (fx, fy): (usize, usize),
    (ox, oy): (f64, f64),
) -> Vec<f64> {
    let taps = |p: usize, origin: f64, factor: usize, size: usize| -> Vec<(usize, f64)> {
        if factor == 1 {
            return vec![(p.min(size - 1), 1.0)];
        }
        let u = (p as f64 - origin) / factor as f64;
        let base = u.floor() as i64;
        let raw: Vec<(usize, f64)> = (base - 2..=base + 3)
            .map(|i| (i.clamp(0, size as i64 - 1) as usize, lanczos3(u - i as f64)))
            .collect();
        let sum: f64 = raw.iter().map(|t| t.1).sum();
        raw.into_iter().map(|(i, wt)| (i, wt / sum)).collect()
    };
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let ty = taps(y, oy, fy, ch);
        for x in 0..w {
            let tx = taps(x, ox, fx, cw);
            out.push(
                ty.iter()
                    .flat_map(|&(j, wy)| tx.iter().map(move |&(i, wx)| (j, i, wy * wx)))
                    .map(|(j, i, wt)| wt * plane[j * cw + i])
                    .sum(),
            );
        }
    }
    out
}
