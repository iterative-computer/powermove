//! Pixel comparison with the section 25 render parity tolerance (Standard
//! color): per-channel 8-bit difference at most 1 for 99.9% of pixels and at
//! most 4 for every pixel. Values are quantized as an 8-bit sink would see them
//! (`round(clamp(v, 0, 1) * 255)`); NaN on one side only counts as 255.

use half::f16;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Metrics {
    pub pixels: usize,
    /// Pixels whose four f16 channels are bit-identical (NaNs compare equal).
    pub exact_pixels: usize,
    /// Pixels with a quantized channel difference greater than 1.
    pub over_one: usize,
    pub max_diff_8bit: u32,
    pub max_abs_float: f32,
    pub nan_mismatch: usize,
    pub pass: bool,
}

fn quantize(v: f32) -> i32 {
    if v.is_nan() {
        return -1;
    }
    (v.clamp(0.0, 1.0) * 255.0).round() as i32
}

pub fn compare(a: &[f16], b: &[f16]) -> Metrics {
    assert_eq!(a.len(), b.len());
    let pixels = a.len() / 4;
    let mut m = Metrics { pixels, exact_pixels: 0, over_one: 0, max_diff_8bit: 0, max_abs_float: 0.0, nan_mismatch: 0, pass: false };
    for p in 0..pixels {
        let mut exact = true;
        let mut worst = 0u32;
        for c in 0..4 {
            let (x, y) = (a[p * 4 + c], b[p * 4 + c]);
            let (fx, fy) = (x.to_f32(), y.to_f32());
            let same_bits = x.to_bits() == y.to_bits() || (fx.is_nan() && fy.is_nan()) || (fx == 0.0 && fy == 0.0);
            if !same_bits {
                exact = false;
            }
            let (qx, qy) = (quantize(fx), quantize(fy));
            let diff = if (qx < 0) != (qy < 0) {
                m.nan_mismatch += 1;
                255
            } else if qx < 0 {
                0
            } else {
                qx.abs_diff(qy)
            };
            worst = worst.max(diff);
            if fx.is_finite() && fy.is_finite() {
                m.max_abs_float = m.max_abs_float.max((fx - fy).abs());
            }
        }
        if exact {
            m.exact_pixels += 1;
        }
        if worst > 1 {
            m.over_one += 1;
        }
        m.max_diff_8bit = m.max_diff_8bit.max(worst);
    }
    m.pass = m.max_diff_8bit <= 4 && (m.over_one as f64) <= 0.001 * pixels as f64;
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(values: &[f32]) -> Vec<f16> {
        values.iter().map(|v| f16::from_f32(*v)).collect()
    }

    #[test]
    fn tolerance_matches_section_25() {
        let n = 2000;
        let a = px(&vec![0.2; n * 4]);
        let mut b = a.clone();
        b[0] = f16::from_f32(0.2 + 3.0 / 255.0); // one pixel off by 3: allowed (1 of 2000 = 0.05%)
        let m = compare(&a, &b);
        assert!(m.pass, "{m:?}");
        b[4] = f16::from_f32(0.2 + 2.0 / 255.0); // a second pixel over 1: 0.1% exceeded
        b[8] = f16::from_f32(0.2 + 2.0 / 255.0);
        assert!(!compare(&a, &b).pass);
        let mut c = a.clone();
        c[0] = f16::from_f32(0.2 + 5.0 / 255.0);
        assert!(!compare(&a, &c).pass, "any pixel above 4 fails");
    }

    #[test]
    fn nan_on_one_side_fails() {
        let a = px(&[0.0, 0.0, 0.0, 1.0]);
        let b = px(&[f32::NAN, 0.0, 0.0, 1.0]);
        let m = compare(&a, &b);
        assert_eq!(m.nan_mismatch, 1);
        assert!(!m.pass);
    }
}
