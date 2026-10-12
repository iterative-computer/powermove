//! CPU reference math: normalization (plan section 13), Rec.709 matrices, the display
//! encoding E (section 9), CIE Lab and ΔE2000, and image metrics.

/// Rec.709 luma coefficients.
pub const KR: f64 = 0.2126;
pub const KB: f64 = 0.0722;
pub const KG: f64 = 1.0 - KR - KB;

/// y416 normalization (section 13): Y′ = (v − 4096) / 56064, C = (v − 32768) / 57344, A = v / 65535.
pub const Y416_Y_OFF: f64 = 4096.0;
pub const Y416_Y_SCALE: f64 = 56064.0;
pub const Y416_C_OFF: f64 = 32768.0;
pub const Y416_C_SCALE: f64 = 57344.0;

/// 10-bit limited-range constants (x420, x422, P010 after c = v / 64).
pub const B10_Y_OFF: f64 = 64.0;
pub const B10_Y_SCALE: f64 = 876.0;
pub const B10_C_OFF: f64 = 512.0;
pub const B10_C_SCALE: f64 = 896.0;

pub fn ycbcr_to_rgb(y: f64, cb: f64, cr: f64) -> [f64; 3] {
    let r = y + 2.0 * (1.0 - KR) * cr;
    let b = y + 2.0 * (1.0 - KB) * cb;
    let g = (y - KR * r - KB * b) / KG;
    [r, g, b]
}

pub fn rgb_to_ycbcr(rgb: [f64; 3]) -> [f64; 3] {
    let y = KR * rgb[0] + KG * rgb[1] + KB * rgb[2];
    let cb = (rgb[2] - y) / (2.0 * (1.0 - KB));
    let cr = (rgb[0] - y) / (2.0 * (1.0 - KR));
    [y, cb, cr]
}

/// The display encoding E: the sRGB piecewise curve extended to all reals.
pub fn e_encode(x: f64) -> f64 {
    let a = x.abs();
    let v = if a <= 0.0031308 { 12.92 * a } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 };
    v.copysign(x)
}

/// E⁻¹.
pub fn e_decode(x: f64) -> f64 {
    let a = x.abs();
    let v = if a <= 0.04045 { a / 12.92 } else { ((a + 0.055) / 1.055).powf(2.4) };
    v.copysign(x)
}

/// Rec.709 / sRGB primaries, D65: linear RGB → XYZ.
pub fn lin709_to_xyz(c: [f64; 3]) -> [f64; 3] {
    [
        0.4123907992659595 * c[0] + 0.357584339383878 * c[1] + 0.1804807884018343 * c[2],
        0.21263900587151036 * c[0] + 0.715168678767756 * c[1] + 0.07219231536073371 * c[2],
        0.01933081871559185 * c[0] + 0.11919477979462599 * c[1] + 0.9505321522496606 * c[2],
    ]
}

/// Display P3 (D65) linear RGB → XYZ.
pub fn linp3_to_xyz(c: [f64; 3]) -> [f64; 3] {
    [
        0.4865709486482162 * c[0] + 0.26566769316909306 * c[1] + 0.1982172852343625 * c[2],
        0.2289745640697488 * c[0] + 0.6917385218365064 * c[1] + 0.079286914093745 * c[2],
        0.0 * c[0] + 0.04511338185890264 * c[1] + 1.043944368900976 * c[2],
    ]
}

pub const D65: [f64; 3] = [0.95047, 1.0, 1.08883];

pub fn xyz_to_lab(xyz: [f64; 3], white: [f64; 3]) -> [f64; 3] {
    fn f(t: f64) -> f64 {
        let d: f64 = 6.0 / 29.0;
        if t > d * d * d { t.cbrt() } else { t / (3.0 * d * d) + 4.0 / 29.0 }
    }
    let fx = f(xyz[0] / white[0]);
    let fy = f(xyz[1] / white[1]);
    let fz = f(xyz[2] / white[2]);
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIEDE2000 (Sharma, Wu, Dalal 2005), kL = kC = kH = 1.
pub fn delta_e2000(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let (l1, a1, b1) = (lab1[0], lab1[1], lab1[2]);
    let (l2, a2, b2) = (lab2[0], lab2[1], lab2[2]);
    let c1 = (a1 * a1 + b1 * b1).sqrt();
    let c2 = (a2 * a2 + b2 * b2).sqrt();
    let cbar = (c1 + c2) / 2.0;
    let cbar7 = cbar.powi(7);
    let g = 0.5 * (1.0 - (cbar7 / (cbar7 + 25f64.powi(7))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let hp = |b: f64, ap: f64| -> f64 {
        if b == 0.0 && ap == 0.0 {
            0.0
        } else {
            let h = b.atan2(ap).to_degrees();
            if h < 0.0 { h + 360.0 } else { h }
        }
    };
    let h1p = hp(b1, a1p);
    let h2p = hp(b2, a2p);
    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let dhp = if c1p * c2p == 0.0 {
        0.0
    } else {
        let d = h2p - h1p;
        if d.abs() <= 180.0 {
            d
        } else if d > 180.0 {
            d - 360.0
        } else {
            d + 360.0
        }
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
    let t = 1.0 - 0.17 * ((hbarp - 30.0) * PI / 180.0).cos()
        + 0.24 * ((2.0 * hbarp) * PI / 180.0).cos()
        + 0.32 * ((3.0 * hbarp + 6.0) * PI / 180.0).cos()
        - 0.20 * ((4.0 * hbarp - 63.0) * PI / 180.0).cos();
    let dtheta = 30.0 * (-((hbarp - 275.0) / 25.0).powi(2)).exp();
    let cbarp7 = cbarp.powi(7);
    let rc = 2.0 * (cbarp7 / (cbarp7 + 25f64.powi(7))).sqrt();
    let lb = (lbarp - 50.0).powi(2);
    let sl = 1.0 + 0.015 * lb / (20.0 + lb).sqrt();
    let sc = 1.0 + 0.045 * cbarp;
    let sh = 1.0 + 0.015 * cbarp * t;
    let rt = -(2.0 * dtheta * PI / 180.0).sin() * rc;
    ((dlp / sl).powi(2) + (dcp / sc).powi(2) + (dhp_big / sh).powi(2) + rt * (dcp / sc) * (dhp_big / sh)).sqrt()
}

/// Statistics of per-sample differences between two integer planes.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DiffStats {
    pub samples: u64,
    pub max_abs: i64,
    pub mismatches: u64,
    pub sum_sq: f64,
}

impl DiffStats {
    pub fn add(&mut self, a: i64, b: i64) {
        let d = a - b;
        self.samples += 1;
        if d != 0 {
            self.mismatches += 1;
            self.max_abs = self.max_abs.max(d.abs());
            self.sum_sq += (d * d) as f64;
        }
    }

    pub fn merge(&mut self, o: &DiffStats) {
        self.samples += o.samples;
        self.mismatches += o.mismatches;
        self.max_abs = self.max_abs.max(o.max_abs);
        self.sum_sq += o.sum_sq;
    }

    /// PSNR in dB for a given peak value.
    pub fn psnr(&self, peak: f64) -> f64 {
        if self.sum_sq == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (peak * peak / (self.sum_sq / self.samples as f64)).log10()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_round_trip() {
        for &(y, cb, cr) in &[(0.5, 0.1, -0.2), (0.0, 0.0, 0.0), (1.0, -0.5, 0.5)] {
            let rgb = ycbcr_to_rgb(y, cb, cr);
            let back = rgb_to_ycbcr(rgb);
            assert!((back[0] - y).abs() < 1e-12);
            assert!((back[1] - cb).abs() < 1e-12);
            assert!((back[2] - cr).abs() < 1e-12);
        }
    }

    #[test]
    fn e_curve_round_trip_and_extension() {
        for i in -100..=200 {
            let x = i as f64 / 100.0;
            assert!((e_decode(e_encode(x)) - x).abs() < 1e-12, "{x}");
        }
        assert!((e_encode(1.0) - 1.0).abs() < 1e-12);
        assert!(e_encode(-0.5) == -e_encode(0.5));
        // 50% linear white encodes to 0.735357 (section 2, linear blending bar).
        assert!((e_encode(0.5) - 0.735357).abs() < 1e-6);
    }

    /// Reference pairs from Sharma, Wu, Dalal (2005), table 1.
    #[test]
    fn delta_e2000_sharma_vectors() {
        let cases: &[([f64; 3], [f64; 3], f64)] = &[
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 3.1571, -77.2803], [50.0, 0.0, -82.7485], 2.8615),
            ([50.0, 2.8361, -74.0200], [50.0, 0.0, -82.7485], 3.4412),
            ([50.0, -1.3802, -84.2814], [50.0, 0.0, -82.7485], 1.0000),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
            ([50.0, 2.5, 0.0], [50.0, 3.2592, 0.3350], 1.0000),
            ([60.2574, -34.0099, 36.2677], [60.4626, -34.1751, 39.4387], 1.2644),
            ([22.7233, 20.0904, -46.6940], [23.0331, 14.9730, -42.5619], 2.0373),
            ([90.8027, -2.0831, 1.4410], [91.1528, -1.6435, 0.0447], 1.4441),
            ([2.0776, 0.0795, -1.1350], [0.9033, -0.0636, -0.5514], 0.9082),
        ];
        for (a, b, want) in cases {
            let got = delta_e2000(*a, *b);
            assert!((got - want).abs() < 1e-4, "{a:?} {b:?}: {got} vs {want}");
        }
    }

    #[test]
    fn psnr_of_identical_is_infinite() {
        let mut d = DiffStats::default();
        d.add(5, 5);
        assert!(d.psnr(1023.0).is_infinite());
        d.add(4, 5);
        assert!(d.psnr(1023.0).is_finite());
    }
}
