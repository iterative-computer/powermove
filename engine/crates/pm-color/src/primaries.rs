//! RGB primaries conversion in linear light (section 9: "P3 stills convert
//! in float, relative colorimetric, no clip until output"; the viewer's
//! Rec.709 → P3 conversion on a `display-p3` canvas).
//!
//! Every supported primaries set has a D65 white, so conversion is the
//! product of two RGB → XYZ matrices with no chromatic adaptation. Values
//! outside the destination gamut stay negative or above 1 (no clipping).

use pm_protocol::color::Primaries;

/// CIE xy chromaticities of red, green, blue, and white.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chromaticities {
    pub r: [f64; 2],
    pub g: [f64; 2],
    pub b: [f64; 2],
    pub w: [f64; 2],
}

const D65: [f64; 2] = [0.3127, 0.3290];

/// The chromaticities of a primaries tag; `None` when unspecified.
pub fn chromaticities(p: Primaries) -> Option<Chromaticities> {
    Some(match p {
        Primaries::Bt709 => Chromaticities {
            r: [0.640, 0.330],
            g: [0.300, 0.600],
            b: [0.150, 0.060],
            w: D65,
        },
        Primaries::DisplayP3 => Chromaticities {
            r: [0.680, 0.320],
            g: [0.265, 0.690],
            b: [0.150, 0.060],
            w: D65,
        },
        Primaries::Bt2020 => Chromaticities {
            r: [0.708, 0.292],
            g: [0.170, 0.797],
            b: [0.131, 0.046],
            w: D65,
        },
        Primaries::Bt601_625 => Chromaticities {
            r: [0.640, 0.330],
            g: [0.290, 0.600],
            b: [0.150, 0.060],
            w: D65,
        },
        Primaries::Bt601_525 => Chromaticities {
            r: [0.630, 0.340],
            g: [0.310, 0.595],
            b: [0.155, 0.070],
            w: D65,
        },
        Primaries::Unspecified => return None,
    })
}

/// A 3×3 matrix, row-major: `out[i] = Σ m[i][j] · in[j]`.
pub type Mat3 = [[f64; 3]; 3];

fn xyz(c: [f64; 2]) -> [f64; 3] {
    [c[0] / c[1], 1.0, (1.0 - c[0] - c[1]) / c[1]]
}

fn inverse(m: &Mat3) -> Mat3 {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let inv = 1.0 / det;
    [
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * inv,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * inv,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * inv,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * inv,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * inv,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * inv,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * inv,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * inv,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * inv,
        ],
    ]
}

fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// Linear RGB → XYZ for a set of chromaticities.
pub fn rgb_to_xyz(c: &Chromaticities) -> Mat3 {
    let (r, g, b) = (xyz(c.r), xyz(c.g), xyz(c.b));
    let m = [[r[0], g[0], b[0]], [r[1], g[1], b[1]], [r[2], g[2], b[2]]];
    let w = xyz(c.w);
    let inv = inverse(&m);
    let s: Vec<f64> = (0..3)
        .map(|i| inv[i][0] * w[0] + inv[i][1] * w[1] + inv[i][2] * w[2])
        .collect();
    [
        [m[0][0] * s[0], m[0][1] * s[1], m[0][2] * s[2]],
        [m[1][0] * s[0], m[1][1] * s[1], m[1][2] * s[2]],
        [m[2][0] * s[0], m[2][1] * s[1], m[2][2] * s[2]],
    ]
}

/// The linear-light conversion between two primaries sets, or `None` when
/// either is unspecified. Identical primaries give the identity exactly.
pub fn conversion(from: Primaries, to: Primaries) -> Option<Mat3> {
    if from == to && from != Primaries::Unspecified {
        return Some([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    }
    let a = rgb_to_xyz(&chromaticities(from)?);
    let b = rgb_to_xyz(&chromaticities(to)?);
    Some(mul(&inverse(&b), &a))
}

/// Applies a row-major matrix.
pub fn apply(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_maps_to_white() {
        for (from, to) in [
            (Primaries::DisplayP3, Primaries::Bt709),
            (Primaries::Bt709, Primaries::DisplayP3),
            (Primaries::Bt2020, Primaries::Bt709),
        ] {
            let m = conversion(from, to).unwrap();
            let w = apply(&m, [1.0, 1.0, 1.0]);
            for c in w {
                assert!((c - 1.0).abs() < 1e-12, "{from:?}→{to:?} {w:?}");
            }
        }
    }

    #[test]
    fn rec709_luminance_row() {
        // The Y row of Rec.709 → XYZ is the familiar luma weights.
        let m = rgb_to_xyz(&chromaticities(Primaries::Bt709).unwrap());
        assert!((m[1][0] - 0.2126).abs() < 1e-4);
        assert!((m[1][1] - 0.7152).abs() < 1e-4);
        assert!((m[1][2] - 0.0722).abs() < 1e-4);
    }

    #[test]
    fn p3_red_is_outside_rec709_and_not_clipped() {
        let m = conversion(Primaries::DisplayP3, Primaries::Bt709).unwrap();
        let red = apply(&m, [1.0, 0.0, 0.0]);
        assert!(red[0] > 1.0 && red[1] < 0.0 && red[2] < 0.0, "{red:?}");
        let back = apply(
            &conversion(Primaries::Bt709, Primaries::DisplayP3).unwrap(),
            red,
        );
        assert!((back[0] - 1.0).abs() < 1e-12 && back[1].abs() < 1e-12);
    }

    #[test]
    fn identity_is_exact() {
        assert_eq!(
            conversion(Primaries::Bt709, Primaries::Bt709).unwrap(),
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
        assert!(conversion(Primaries::Unspecified, Primaries::Bt709).is_none());
    }
}
