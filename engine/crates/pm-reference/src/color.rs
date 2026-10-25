//! Color transforms in f64, written from the standards:
//! IEC 61966-2-1 (sRGB), Rec. ITU-R BT.709, BT.2020 and BT.2100 (PQ, HLG),
//! SMPTE ST 2084, Rec. ITU-T H.273 (code points for primaries, transfer,
//! matrix coefficients and range), SMPTE RP 177 (RGB to XYZ from
//! primaries), and ICC.1:2022 (matrix/TRC profiles, the D50 PCS, Bradford
//! adaptation).

/// A 3×3 matrix, row-major.
pub type M3 = [[f64; 3]; 3];

pub fn apply(m: &M3, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

pub fn mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

pub fn invert(m: &M3) -> M3 {
    let c = |r: usize, k: usize| {
        let (r1, r2, k1, k2) = ((r + 1) % 3, (r + 2) % 3, (k + 1) % 3, (k + 2) % 3);
        m[r1][k1] * m[r2][k2] - m[r1][k2] * m[r2][k1]
    };
    let det: f64 = (0..3).map(|k| m[0][k] * c(0, k)).sum();
    std::array::from_fn(|r| std::array::from_fn(|k| c(k, r) / det))
}

/// Transfer characteristics (H.273 Table 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transfer {
    /// 8: linear.
    Linear,
    /// 13: IEC 61966-2-1 sRGB.
    Srgb,
    /// 1, 6, 14, 15: the BT.709 OETF (scene-referred). [`Self::from_h273`]
    /// gives this; video displayed on a reference monitor is decoded with
    /// [`Transfer::Bt1886`] instead.
    Bt709,
    /// BT.1886 EOTF with a zero black level (γ 2.4): the display-referred
    /// decode of BT.709 video.
    Bt1886,
    /// 4 (2.2), 5 (2.8): a pure power law.
    Gamma(f64),
    /// 16: SMPTE ST 2084; linear is in units of 10,000 cd/m².
    Pq,
    /// 18: BT.2100 HLG; linear is normalized scene light.
    Hlg,
}

const BT709_ALPHA: f64 = 1.099_296_826_809_442;
const BT709_BETA: f64 = 0.018_053_968_510_807;
const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;
const HLG_A: f64 = 0.178_832_77;
const HLG_B: f64 = 1.0 - 4.0 * HLG_A;

fn hlg_c() -> f64 {
    0.5 - HLG_A * (4.0 * HLG_A).ln()
}

/// Odd extension of a transfer function to negative values.
fn odd(v: f64, f: impl Fn(f64) -> f64) -> f64 {
    if v < 0.0 { -f(-v) } else { f(v) }
}

impl Transfer {
    pub fn from_h273(code: u8) -> Option<Transfer> {
        Some(match code {
            1 | 6 | 14 | 15 => Transfer::Bt709,
            4 => Transfer::Gamma(2.2),
            5 => Transfer::Gamma(2.8),
            8 => Transfer::Linear,
            13 => Transfer::Srgb,
            16 => Transfer::Pq,
            18 => Transfer::Hlg,
            _ => return None,
        })
    }

    /// Encoded value to linear light: the EOTF for sRGB, power laws and
    /// PQ; the inverse OETF for BT.709 and HLG (scene light).
    pub fn to_linear(self, v: f64) -> f64 {
        match self {
            Transfer::Linear => v,
            Transfer::Srgb => odd(v, |v| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            }),
            Transfer::Bt709 => odd(v, |v| {
                if v < 4.5 * BT709_BETA {
                    v / 4.5
                } else {
                    ((v + BT709_ALPHA - 1.0) / BT709_ALPHA).powf(1.0 / 0.45)
                }
            }),
            Transfer::Gamma(g) => odd(v, |v| v.powf(g)),
            Transfer::Bt1886 => odd(v, |v| v.powf(2.4)),
            Transfer::Pq => {
                let p = v.max(0.0).powf(1.0 / PQ_M2);
                ((p - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * p)).powf(1.0 / PQ_M1)
            }
            Transfer::Hlg => {
                let v = v.max(0.0);
                if v <= 0.5 {
                    v * v / 3.0
                } else {
                    (((v - hlg_c()) / HLG_A).exp() + HLG_B) / 12.0
                }
            }
        }
    }

    /// Linear light to the encoded value (the inverse of [`Self::to_linear`]).
    pub fn from_linear(self, l: f64) -> f64 {
        match self {
            Transfer::Linear => l,
            Transfer::Srgb => odd(l, |l| {
                if l <= 0.003_130_8 {
                    12.92 * l
                } else {
                    1.055 * l.powf(1.0 / 2.4) - 0.055
                }
            }),
            Transfer::Bt709 => odd(l, |l| {
                if l < BT709_BETA {
                    4.5 * l
                } else {
                    BT709_ALPHA * l.powf(0.45) - (BT709_ALPHA - 1.0)
                }
            }),
            Transfer::Gamma(g) => odd(l, |l| l.powf(1.0 / g)),
            Transfer::Bt1886 => odd(l, |l| l.powf(1.0 / 2.4)),
            Transfer::Pq => {
                let y = l.max(0.0).powf(PQ_M1);
                ((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y)).powf(PQ_M2)
            }
            Transfer::Hlg => {
                let l = l.max(0.0);
                if l <= 1.0 / 12.0 {
                    (3.0 * l).sqrt()
                } else {
                    HLG_A * (12.0 * l - HLG_B).ln() + hlg_c()
                }
            }
        }
    }
}

/// BT.2100 HLG OOTF: normalized scene light (BT.2020 primaries) to display
/// light relative to the nominal peak `peak_nits` (black level 0).
pub fn hlg_ootf(scene: [f64; 3], peak_nits: f64) -> [f64; 3] {
    let gamma = 1.2 + 0.42 * (peak_nits / 1000.0).log10();
    let ys = 0.2627 * scene[0] + 0.6780 * scene[1] + 0.0593 * scene[2];
    let k = if ys > 0.0 { ys.powf(gamma - 1.0) } else { 0.0 };
    scene.map(|e| k * e)
}

/// Matrix coefficients (H.273 Table 4) as (KR, KB); `None` for identity
/// (GBR, code 0).
pub fn matrix_coefficients(code: u8) -> Option<Option<(f64, f64)>> {
    Some(match code {
        0 => None,
        1 => Some((0.2126, 0.0722)),
        4 => Some((0.30, 0.11)),
        5 | 6 => Some((0.299, 0.114)),
        7 => Some((0.212, 0.087)),
        9 => Some((0.2627, 0.0593)),
        _ => return None,
    })
}

/// E′Y, E′Pb, E′Pr (Y in 0…1, Pb and Pr in −½…½) to E′R, E′G, E′B (H.273
/// equations 38–40 solved for R, G, B).
pub fn ycbcr_to_rgb(ypbpr: [f64; 3], (kr, kb): (f64, f64)) -> [f64; 3] {
    let [y, pb, pr] = ypbpr;
    let r = y + 2.0 * (1.0 - kr) * pr;
    let b = y + 2.0 * (1.0 - kb) * pb;
    let g = (y - kr * r - kb * b) / (1.0 - kr - kb);
    [r, g, b]
}

/// E′R, E′G, E′B to E′Y, E′Pb, E′Pr (H.273 equations 38–40).
pub fn rgb_to_ycbcr(rgb: [f64; 3], (kr, kb): (f64, f64)) -> [f64; 3] {
    let y = kr * rgb[0] + (1.0 - kr - kb) * rgb[1] + kb * rgb[2];
    [y, (rgb[2] - y) / (2.0 * (1.0 - kb)), (rgb[0] - y) / (2.0 * (1.0 - kr))]
}

/// A luma (or, for matrix 0, any R′G′B′) code value of `bits` bits to its
/// normalized value (H.273 equations 20–25, without rounding).
pub fn dequantize_luma(code: f64, bits: u32, full_range: bool) -> f64 {
    if full_range {
        code / (2f64.powi(bits as i32) - 1.0)
    } else {
        (code / 2f64.powi(bits as i32 - 8) - 16.0) / 219.0
    }
}

/// A chroma code value to its normalized value in −½…½.
pub fn dequantize_chroma(code: f64, bits: u32, full_range: bool) -> f64 {
    if full_range {
        (code - 2f64.powi(bits as i32 - 1)) / (2f64.powi(bits as i32) - 1.0)
    } else {
        (code / 2f64.powi(bits as i32 - 8) - 128.0) / 224.0
    }
}

/// Color primaries and white point as CIE 1931 xy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primaries {
    pub r: [f64; 2],
    pub g: [f64; 2],
    pub b: [f64; 2],
    pub white: [f64; 2],
}

const D65: [f64; 2] = [0.3127, 0.3290];

impl Primaries {
    /// 1: BT.709 and sRGB.
    pub const BT709: Primaries = Primaries {
        r: [0.640, 0.330],
        g: [0.300, 0.600],
        b: [0.150, 0.060],
        white: D65,
    };
    /// 5: BT.601 625-line (BT.470 System B, G).
    pub const BT601_625: Primaries = Primaries {
        r: [0.640, 0.330],
        g: [0.290, 0.600],
        b: [0.150, 0.060],
        white: D65,
    };
    /// 6: BT.601 525-line (SMPTE 170M).
    pub const BT601_525: Primaries = Primaries {
        r: [0.630, 0.340],
        g: [0.310, 0.595],
        b: [0.155, 0.070],
        white: D65,
    };
    /// 9: BT.2020 and BT.2100.
    pub const BT2020: Primaries = Primaries {
        r: [0.708, 0.292],
        g: [0.170, 0.797],
        b: [0.131, 0.046],
        white: D65,
    };
    /// 12: SMPTE EG 432-1 (Display P3).
    pub const P3_D65: Primaries = Primaries {
        r: [0.680, 0.320],
        g: [0.265, 0.690],
        b: [0.150, 0.060],
        white: D65,
    };

    pub fn from_h273(code: u8) -> Option<Primaries> {
        Some(match code {
            1 => Primaries::BT709,
            5 => Primaries::BT601_625,
            6 | 7 => Primaries::BT601_525,
            9 => Primaries::BT2020,
            12 => Primaries::P3_D65,
            _ => return None,
        })
    }

    /// Linear RGB to XYZ (Y of white = 1), SMPTE RP 177.
    pub fn to_xyz(&self) -> M3 {
        let col = |[x, y]: [f64; 2]| [x / y, 1.0, (1.0 - x - y) / y];
        let (r, g, b) = (col(self.r), col(self.g), col(self.b));
        let p = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
        let s = apply(&invert(&p), col(self.white));
        std::array::from_fn(|i| std::array::from_fn(|j| p[i][j] * s[j]))
    }
}

/// XYZ of an xy white with Y = 1.
pub fn white_xyz([x, y]: [f64; 2]) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// The ICC PCS illuminant (ICC.1:2022 §7.2.16), D50 as XYZ.
pub const D50_XYZ: [f64; 3] = [0.9642, 1.0, 0.8249];

/// Bradford chromatic adaptation from white `src` to white `dst` (XYZ),
/// ICC.1:2022 Annex E.
pub fn bradford(src: [f64; 3], dst: [f64; 3]) -> M3 {
    const B: M3 = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let (s, d) = (apply(&B, src), apply(&B, dst));
    let scale = [[d[0] / s[0], 0.0, 0.0], [0.0, d[1] / s[1], 0.0], [0.0, 0.0, d[2] / s[2]]];
    mul(&invert(&B), &mul(&scale, &B))
}

/// A tone reproduction curve (ICC `curv` and `para` types).
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Gamma(f64),
    /// Samples at `i / (n − 1)`, linearly interpolated.
    Table(Vec<f64>),
    /// `para` function types 0–4 with parameters g, a, b, c, d, e, f.
    Parametric(u16, [f64; 7]),
}

impl Curve {
    /// The IEC 61966-2-1 curve as ICC `para` type 3.
    pub fn srgb() -> Curve {
        Curve::Parametric(3, [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045, 0.0, 0.0])
    }

    pub fn eval(&self, x: f64) -> f64 {
        match self {
            Curve::Gamma(g) => x.max(0.0).powf(*g),
            Curve::Table(t) => {
                let n = t.len() - 1;
                let p = x.clamp(0.0, 1.0) * n as f64;
                let i = (p.floor() as usize).min(n.saturating_sub(1));
                let f = p - i as f64;
                if n == 0 { t[0] } else { t[i] + (t[i + 1] - t[i]) * f }
            }
            Curve::Parametric(kind, [g, a, b, c, d, e, f]) => {
                let pow = |x: f64| (a * x + b).max(0.0).powf(*g);
                match kind {
                    0 => x.max(0.0).powf(*g),
                    1 => if x >= -b / a { pow(x) } else { 0.0 },
                    2 => if x >= -b / a { pow(x) + c } else { *c },
                    3 => if x >= *d { pow(x) } else { c * x },
                    _ => if x >= *d { pow(x) + e } else { c * x + f },
                }
            }
        }
    }

    /// [`Self::eval`] for any real: odd-symmetric below 0; gamma and
    /// parametric curves continue their formula above 1, tables their last
    /// slope.
    pub fn eval_extended(&self, x: f64) -> f64 {
        match self {
            _ if x < 0.0 => -self.eval_extended(-x),
            Curve::Table(t) if x > 1.0 && t.len() > 1 => {
                let n = t.len() - 1;
                t[n] + (x - 1.0) * n as f64 * (t[n] - t[n - 1])
            }
            _ => self.eval(x),
        }
    }

    /// The inverse of a monotonic curve: on 0…1 (`Extent::Clip`), or of
    /// [`Self::eval_extended`] for any real (`Extent::Unclipped`).
    pub fn invert(&self, y: f64, extent: Extent) -> f64 {
        if extent == Extent::Unclipped && y < 0.0 {
            return -self.invert(-y, extent);
        }
        if let Curve::Gamma(g) = self {
            let x = y.max(0.0).powf(1.0 / g);
            return if extent == Extent::Clip { x.min(1.0) } else { x };
        }
        let f = |x: f64| match extent {
            Extent::Clip => self.eval(x),
            Extent::Unclipped => self.eval_extended(x),
        };
        let mut hi = 1.0f64;
        while extent == Extent::Unclipped && f(hi) < y && hi < 1e12 {
            hi *= 2.0;
        }
        let mut lo = 0.0f64;
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if f(mid) < y { lo = mid } else { hi = mid }
        }
        0.5 * (lo + hi)
    }
}

/// What a conversion does with values outside 0…1 (out of gamut, above
/// white).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    /// Curves are defined on 0…1 only (ICC.1 integer pipelines).
    Clip,
    /// Kept: curves extended odd-symmetrically and beyond 1, as float
    /// pipelines keep such values until output.
    Unclipped,
}

/// An RGB matrix/TRC profile: device RGB → TRC → linear RGB → XYZ (D50 PCS).
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixTrc {
    pub to_pcs: M3,
    pub trc: [Curve; 3],
}

impl MatrixTrc {
    /// A profile from primaries and one curve, adapted to D50 with Bradford.
    pub fn from_primaries(p: &Primaries, curve: Curve) -> MatrixTrc {
        let adapt = bradford(white_xyz(p.white), D50_XYZ);
        MatrixTrc {
            to_pcs: mul(&adapt, &p.to_xyz()),
            trc: [curve.clone(), curve.clone(), curve],
        }
    }

    /// Reads an ICC RGB profile with `rXYZ`/`gXYZ`/`bXYZ` and
    /// `rTRC`/`gTRC`/`bTRC` tags (ICC.1:2022 §8.4).
    pub fn parse_icc(icc: &[u8]) -> Result<MatrixTrc, String> {
        let u32_at = |o: usize| -> Result<u32, String> {
            icc.get(o..o + 4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| "truncated profile".to_owned())
        };
        let fixed = |o: usize| u32_at(o).map(|v| f64::from(v as i32) / 65536.0);
        if icc.get(16..20) != Some(b"RGB ") || icc.get(20..24) != Some(b"XYZ ") {
            return Err("not an RGB profile with an XYZ PCS".into());
        }
        let count = u32_at(128)? as usize;
        let tag = |sig: &[u8; 4]| -> Result<usize, String> {
            (0..count)
                .map(|i| 132 + 12 * i)
                .find(|&e| icc.get(e..e + 4) == Some(sig))
                .map(|e| u32_at(e + 4).map(|o| o as usize))
                .unwrap_or_else(|| Err(format!("no {} tag", String::from_utf8_lossy(sig))))
        };
        let xyz = |sig: &[u8; 4]| -> Result<[f64; 3], String> {
            let o = tag(sig)?;
            if icc.get(o..o + 4) != Some(b"XYZ ") {
                return Err("bad XYZ tag".into());
            }
            Ok([fixed(o + 8)?, fixed(o + 12)?, fixed(o + 16)?])
        };
        let curve = |sig: &[u8; 4]| -> Result<Curve, String> {
            let o = tag(sig)?;
            match icc.get(o..o + 4) {
                Some(b"curv") => {
                    let n = u32_at(o + 8)? as usize;
                    let entry = |i: usize| -> Result<f64, String> {
                        icc.get(o + 12 + 2 * i..o + 14 + 2 * i)
                            .map(|b| f64::from(u16::from_be_bytes([b[0], b[1]])))
                            .ok_or_else(|| "truncated curve".to_owned())
                    };
                    Ok(match n {
                        0 => Curve::Gamma(1.0),
                        1 => Curve::Gamma(entry(0)? / 256.0),
                        _ => Curve::Table(
                            (0..n).map(|i| entry(i).map(|v| v / 65535.0)).collect::<Result<_, _>>()?,
                        ),
                    })
                }
                Some(b"para") => {
                    let kind = (u32_at(o + 8)? >> 16) as u16;
                    let n = [1, 3, 4, 5, 7].get(kind as usize).ok_or("bad para type")?;
                    let mut p = [0.0; 7];
                    for (i, v) in p.iter_mut().take(*n).enumerate() {
                        *v = fixed(o + 12 + 4 * i)?;
                    }
                    Ok(Curve::Parametric(kind, p))
                }
                _ => Err("unsupported TRC type".into()),
            }
        };
        let (r, g, b) = (xyz(b"rXYZ")?, xyz(b"gXYZ")?, xyz(b"bXYZ")?);
        Ok(MatrixTrc {
            to_pcs: [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]],
            trc: [curve(b"rTRC")?, curve(b"gTRC")?, curve(b"bTRC")?],
        })
    }

    pub fn to_pcs(&self, rgb: [f64; 3], extent: Extent) -> [f64; 3] {
        apply(
            &self.to_pcs,
            std::array::from_fn(|i| match extent {
                Extent::Clip => self.trc[i].eval(rgb[i].clamp(0.0, 1.0)),
                Extent::Unclipped => self.trc[i].eval_extended(rgb[i]),
            }),
        )
    }

    pub fn from_pcs(&self, xyz: [f64; 3], extent: Extent) -> [f64; 3] {
        let linear = apply(&invert(&self.to_pcs), xyz);
        std::array::from_fn(|i| self.trc[i].invert(linear[i], extent))
    }
}

/// `rgb` in profile `src` to profile `dst` (relative colorimetric through
/// the PCS), clipped to 0…1 or not.
pub fn convert(src: &MatrixTrc, dst: &MatrixTrc, rgb: [f64; 3], extent: Extent) -> [f64; 3] {
    dst.from_pcs(src.to_pcs(rgb, extent), extent)
}

/// The BT.2390 EETF (Report ITU-R BT.2390, §5.4.1) on a PQ signal, from
/// content mastered at `source_peak` cd/m² to a display peaking at
/// `target_peak` cd/m², black at zero (no toe): identity below the knee
/// KS = 1.5·maxLum − 0.5, a Hermite spline to maxLum above it.
pub fn eetf_bt2390(e: f64, source_peak: f64, target_peak: f64) -> f64 {
    let pw = Transfer::Pq.from_linear(source_peak / 10_000.0);
    let max_lum = Transfer::Pq.from_linear(target_peak / 10_000.0) / pw;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfers_round_trip() {
        for t in [
            Transfer::Srgb,
            Transfer::Bt709,
            Transfer::Bt1886,
            Transfer::Gamma(2.2),
            Transfer::Pq,
            Transfer::Hlg,
        ] {
            // PQ code 0 is below the code of zero light (c1^m2).
            for i in 1..=100 {
                let v = f64::from(i) / 100.0;
                let back = t.from_linear(t.to_linear(v));
                assert!((back - v).abs() < 1e-12, "{t:?} {v} {back}");
            }
        }
        // Reference points of the standards.
        assert!((Transfer::Pq.to_linear(1.0) - 1.0).abs() < 1e-12);
        assert!((Transfer::Pq.from_linear(100.0 / 10000.0) - 0.508_078).abs() < 1e-6);
        assert!((Transfer::Hlg.from_linear(1.0 / 12.0) - 0.5).abs() < 1e-12);
        assert!((Transfer::Hlg.from_linear(1.0) - 1.0).abs() < 1e-7);
        assert!((Transfer::Srgb.to_linear(0.5) - 0.214_041_140_5).abs() < 1e-9);
    }

    #[test]
    fn bt709_primaries_give_the_published_matrix() {
        // IEC 61966-2-1 / BT.709 RGB→XYZ (D65), rounded as published.
        let m = Primaries::BT709.to_xyz();
        let want = [
            [0.4124, 0.3576, 0.1805],
            [0.2126, 0.7152, 0.0722],
            [0.0193, 0.1192, 0.9505],
        ];
        for (r, row) in want.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                assert!((m[r][c] - v).abs() < 1e-4, "{r} {c}: {}", m[r][c]);
            }
        }
        // Luma weights are the Y row (BT.709 KR, KB).
        assert!((m[1][0] - 0.2126).abs() < 1e-4 && (m[1][2] - 0.0722).abs() < 1e-4);
    }

    #[test]
    fn ycbcr_round_trip_and_ranges() {
        let k = matrix_coefficients(1).unwrap().unwrap();
        let rgb = [0.8, 0.3, 0.1];
        let back = ycbcr_to_rgb(rgb_to_ycbcr(rgb, k), k);
        assert!(rgb.iter().zip(back).all(|(a, b)| (a - b).abs() < 1e-14));
        assert_eq!(dequantize_luma(16.0, 8, false), 0.0);
        assert_eq!(dequantize_luma(940.0, 10, false), 1.0);
        assert_eq!(dequantize_chroma(512.0, 10, false), 0.0);
        assert_eq!(dequantize_luma(1023.0, 10, true), 1.0);
    }

    #[test]
    fn srgb_profile_matches_the_icc_srgb_colorants() {
        // ICC sRGB v4 colorants (D50-adapted, as in sRGB_v4_ICC_preference).
        let p = MatrixTrc::from_primaries(&Primaries::BT709, Curve::srgb());
        let want = [
            [0.4361, 0.3851, 0.1431],
            [0.2225, 0.7169, 0.0606],
            [0.0139, 0.0971, 0.7141],
        ];
        for (r, row) in want.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                assert!((p.to_pcs[r][c] - v).abs() < 2e-4, "{r} {c}");
            }
        }
        let rgb = [0.25, 0.5, 0.75];
        let back = convert(&p, &p, rgb, Extent::Clip);
        assert!(rgb.iter().zip(back).all(|(a, b)| (a - b).abs() < 1e-12));
        assert!((Curve::srgb().eval(0.5) - Transfer::Srgb.to_linear(0.5)).abs() < 1e-15);
    }

    #[test]
    fn wide_gamut_converts_unclipped() {
        // Display P3 red is outside sRGB: negative green and blue, kept
        // unclipped, and it round-trips; clipped it does not.
        let p3 = MatrixTrc::from_primaries(&Primaries::P3_D65, Curve::srgb());
        let srgb = MatrixTrc::from_primaries(&Primaries::BT709, Curve::srgb());
        let red = convert(&p3, &srgb, [1.0, 0.0, 0.0], Extent::Unclipped);
        assert!(red[0] > 1.0 && red[1] < 0.0 && red[2] < 0.0, "{red:?}");
        let back = convert(&srgb, &p3, red, Extent::Unclipped);
        assert!(back.iter().zip([1.0, 0.0, 0.0]).all(|(a, b)| (a - b).abs() < 1e-9), "{back:?}");
        let clipped = convert(&p3, &srgb, [1.0, 0.0, 0.0], Extent::Clip);
        assert!(clipped.iter().all(|v| (0.0..=1.0).contains(v)));
        let table = Curve::Table(vec![0.0, 0.25, 1.0]);
        assert!((table.invert(table.eval_extended(1.5), Extent::Unclipped) - 1.5).abs() < 1e-9);
    }

    #[test]
    fn parses_a_matrix_trc_profile() {
        // A minimal v2 profile: XYZ colorants and a gamma 2.2 `curv`.
        let mut icc = vec![0u8; 128];
        icc[16..20].copy_from_slice(b"RGB ");
        icc[20..24].copy_from_slice(b"XYZ ");
        let tags: [&[u8; 4]; 6] = [b"rXYZ", b"gXYZ", b"bXYZ", b"rTRC", b"gTRC", b"bTRC"];
        icc.extend_from_slice(&(tags.len() as u32).to_be_bytes());
        let table = icc.len();
        icc.resize(table + 12 * tags.len(), 0);
        let fixed = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
        let cols = [[0.4361, 0.2225, 0.0139], [0.3851, 0.7169, 0.0971], [0.1431, 0.0606, 0.7141]];
        for (i, sig) in tags.iter().enumerate() {
            let offset = icc.len() as u32;
            if i < 3 {
                icc.extend_from_slice(b"XYZ \0\0\0\0");
                cols[i].iter().for_each(|v| icc.extend_from_slice(&fixed(*v)));
            } else {
                icc.extend_from_slice(b"curv\0\0\0\0");
                icc.extend_from_slice(&1u32.to_be_bytes());
                icc.extend_from_slice(&((2.2f64 * 256.0).round() as u16).to_be_bytes());
                icc.extend_from_slice(&[0, 0]);
            }
            let e = table + 12 * i;
            icc[e..e + 4].copy_from_slice(*sig);
            icc[e + 4..e + 8].copy_from_slice(&offset.to_be_bytes());
            let size = icc.len() as u32 - offset;
            icc[e + 8..e + 12].copy_from_slice(&size.to_be_bytes());
        }
        let p = MatrixTrc::parse_icc(&icc).unwrap();
        assert!((p.to_pcs[1][1] - 0.7169).abs() < 1e-4);
        assert_eq!(p.trc[0], Curve::Gamma(563.0 / 256.0));
        let white = p.to_pcs([1.0; 3], Extent::Clip);
        assert!((white[1] - 1.0).abs() < 1e-3);
    }
}
