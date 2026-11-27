//! Section 25's render parity classes as measurements.

/// Float to 8-bit as a unorm render-target write does: clamp, scale, round
/// to nearest even.
pub fn unorm8(v: f32) -> u8 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    (v * 255.0).round_ties_even() as u8
}

/// The capture's opaque view of premultiplied working RGBA: color over black,
/// alpha 255 (`FRAG_CAPTURE_OPAQUE` into RGBA8).
pub fn opaque8(rgba: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&[unorm8(px[0]), unorm8(px[1]), unorm8(px[2]), 255]);
    }
    out
}

/// Premultiplied float RGBA to 8-bit RGBA (alpha kept).
pub fn rgba8(rgba: &[f32]) -> Vec<u8> {
    rgba.iter().map(|v| unorm8(*v)).collect()
}

/// Standard color: per-channel 8-bit difference ≤ 1 for 99.9% of pixels, ≤ 4
/// for any pixel.
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    pub pixels: usize,
    /// Pixels whose largest channel difference is ≤ 1.
    pub within_one: usize,
    pub max: u8,
    pub exact: usize,
}

impl Diff {
    /// Over every pixel (`mask` `None`) or the masked ones.
    pub fn of(a: &[u8], b: &[u8], mask: Option<&[bool]>) -> Diff {
        assert_eq!(a.len(), b.len());
        let mut d = Diff::default();
        for (i, (p, q)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
            if mask.is_some_and(|m| !m[i]) {
                continue;
            }
            let m = (0..4).map(|c| p[c].abs_diff(q[c])).max().unwrap_or(0);
            d.pixels += 1;
            if m <= 1 {
                d.within_one += 1;
            }
            if m == 0 {
                d.exact += 1;
            }
            d.max = d.max.max(m);
        }
        d
    }

    pub fn fraction_within_one(&self) -> f64 {
        self.within_one as f64 / self.pixels.max(1) as f64
    }

    pub fn passes(&self) -> bool {
        self.fraction_within_one() >= 0.999 && self.max <= 4
    }
}

/// Pixels where the reference's 3×3 neighborhood is uniform within 1 CV
/// (patch interiors, section 25's video class).
pub fn interior(reference: &[u8], width: u32, height: u32) -> Vec<bool> {
    let (w, h) = (i64::from(width), i64::from(height));
    let at = |x: i64, y: i64, c: usize| reference[((y * w + x) * 4) as usize + c];
    let mut out = vec![false; (w * h) as usize];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let mut uniform = true;
            'n: for dy in -1..=1 {
                for dx in -1..=1 {
                    for c in 0..3 {
                        if at(x + dx, y + dy, c).abs_diff(at(x, y, c)) > 1 {
                            uniform = false;
                            break 'n;
                        }
                    }
                }
            }
            out[(y * w + x) as usize] = uniform;
        }
    }
    out
}

/// The largest color difference over masked pixels, and how many.
pub fn masked_max(a: &[u8], b: &[u8], mask: &[bool]) -> (u8, usize) {
    let mut max = 0;
    let mut n = 0;
    for (i, &m) in mask.iter().enumerate() {
        if !m {
            continue;
        }
        n += 1;
        for c in 0..3 {
            max = max.max(a[i * 4 + c].abs_diff(b[i * 4 + c]));
        }
    }
    (max, n)
}

/// The largest difference between two float frames over masked pixels (all
/// with `None`), in 8-bit code values.
pub fn float_max_cv(a: &[f32], b: &[f32], mask: Option<&[bool]>) -> f64 {
    assert_eq!(a.len(), b.len());
    let mut max = 0.0f64;
    for (i, (p, q)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        if mask.is_some_and(|m| !m[i]) {
            continue;
        }
        for c in 0..4 {
            max = max.max((f64::from(p[c]) - f64::from(q[c])).abs() * 255.0);
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_class_thresholds() {
        let a = [10u8, 10, 10, 255].repeat(1000);
        let mut b = a.clone();
        b[0] = 11;
        let d = Diff::of(&a, &b, None);
        assert_eq!(
            (d.pixels, d.within_one, d.exact, d.max),
            (1000, 1000, 999, 1)
        );
        assert!(d.passes());
        b[4] = 15;
        let d = Diff::of(&a, &b, None);
        assert_eq!(d.max, 5);
        assert!(!d.passes(), "any pixel above 4");
        b[4] = 12;
        b[8] = 12;
        let d = Diff::of(&a, &b, None);
        assert!(!d.passes(), "2 of 1000 above 1 is under 99.9%");
        let mask: Vec<bool> = (0..1000).map(|i| i != 1 && i != 2).collect();
        assert!(Diff::of(&a, &b, Some(&mask)).passes());
    }

    #[test]
    fn interiors_skip_edges_and_detail() {
        let (w, h) = (5u32, 4u32);
        let mut img = [100u8, 100, 100, 255].repeat((w * h) as usize);
        img[(2 * 5 + 2) * 4] = 160;
        let m = interior(&img, w, h);
        assert!(!m[0], "the border is not an interior");
        assert!(
            !m[2 * 5 + 2] && !m[5 + 1],
            "the bright pixel and its neighbors"
        );
        let (max, n) = masked_max(&img, &img, &m);
        assert_eq!(max, 0);
        assert_eq!(n, m.iter().filter(|v| **v).count());
    }

    #[test]
    fn unorm_rounding_matches_a_render_target() {
        assert_eq!(opaque8(&[0.5, -1.0, 2.0, 0.0]), vec![128, 0, 255, 255]);
        assert_eq!(rgba8(&[f32::NAN, 1.0]), vec![0, 255]);
        let d = float_max_cv(&[0.0, 0.0, 0.0, 1.0], &[0.0, 0.0, 2.0 / 255.0, 1.0], None);
        assert!((d - 2.0).abs() < 1e-4, "{d}");
    }
}
