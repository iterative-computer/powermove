//! Cross-checks of the reference against the engine's color library
//! (`pm-color`). The reference never calls `pm-color` for its values; this
//! test only confirms the two independent implementations of the same
//! standards and authored formulas agree, so a disagreement points at a bug
//! in one of them.

use pm_color::linear::{composite, legacy_blend};
use pm_protocol::color::{BlendDomain, BlendMode, MatrixCoefficients, Range, WorkingEncoding};
use pm_reference::color::{self, Primaries, Transfer};
use pm_reference::composite::{Blend, blend, blend_channel};

const MODES: [(BlendMode, Blend); 9] = [
    (BlendMode::Normal, Blend::Normal),
    (BlendMode::Add, Blend::LinearDodge),
    (BlendMode::Screen, Blend::Screen),
    (BlendMode::Multiply, Blend::Multiply),
    (BlendMode::Overlay, Blend::Overlay),
    (BlendMode::SoftLight, Blend::SoftLightPhotoshop),
    (BlendMode::Difference, Blend::Difference),
    (BlendMode::Lighten, Blend::Lighten),
    (BlendMode::Darken, Blend::Darken),
];

fn grid() -> impl Iterator<Item = f64> {
    (0..=20).map(|i| f64::from(i) / 20.0)
}

#[test]
fn blend_modes_agree() {
    for (engine, reference) in MODES {
        for b in grid() {
            for s in grid() {
                let (x, y) = (legacy_blend(engine, b, s), blend_channel(reference, b, s));
                assert!((x - y).abs() < 1e-15, "{engine:?} b {b} s {s}: {x} vs {y}");
            }
        }
    }
}

#[test]
fn premultiplied_composites_agree() {
    let colors = [
        [0.1, 0.5, 0.3, 0.6],
        [0.0, 0.0, 0.0, 0.0],
        [0.9, 0.2, 0.7, 1.0],
        [0.05, 0.1, 0.02, 0.25],
    ];
    for (engine, reference) in MODES {
        for src in colors {
            for dst in colors {
                for alpha in [1.0, 0.5] {
                    let x = composite(
                        engine,
                        WorkingEncoding::Standard,
                        BlendDomain::HdrSafe,
                        src,
                        dst,
                        alpha,
                    );
                    let y = blend(reference, src.map(|v| v * alpha), dst);
                    for i in 0..4 {
                        assert!((x[i] - y[i]).abs() < 1e-12, "{engine:?} {src:?} {dst:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn transfers_agree() {
    for i in -10..=120 {
        let v = f64::from(i) / 100.0;
        // The engine uses BT.709-6's rounded α = 1.099, β = 0.018; the
        // reference H.273's exact constants. They differ by < 2e-4 (0.05 CV).
        let pairs = [
            (pm_color::transfer::display_decode(v), Transfer::Srgb.to_linear(v), 1e-12),
            (pm_color::transfer::display_encode(v), Transfer::Srgb.from_linear(v), 1e-12),
            (pm_color::transfer::bt709_inverse_oetf(v), Transfer::Bt709.to_linear(v), 2e-4),
            (pm_color::transfer::gamma22_decode(v), Transfer::Gamma(2.2).to_linear(v), 1e-12),
        ];
        for (k, (x, y, tolerance)) in pairs.into_iter().enumerate() {
            assert!((x - y).abs() < tolerance, "pair {k} at {v}: {x} vs {y}");
        }
    }
}

#[test]
fn primaries_and_matrices_agree() {
    for (engine, reference) in [
        (pm_protocol::color::Primaries::Bt709, Primaries::BT709),
        (pm_protocol::color::Primaries::DisplayP3, Primaries::P3_D65),
        (pm_protocol::color::Primaries::Bt2020, Primaries::BT2020),
    ] {
        let m = pm_color::primaries::rgb_to_xyz(&pm_color::primaries::chromaticities(engine).unwrap());
        let r = reference.to_xyz();
        for i in 0..3 {
            for j in 0..3 {
                assert!((m[i][j] - r[i][j]).abs() < 1e-12, "{engine:?}");
            }
        }
    }
    for (engine, code) in [
        (MatrixCoefficients::Bt709, 1),
        (MatrixCoefficients::Bt601, 6),
        (MatrixCoefficients::Bt2020Ncl, 9),
    ] {
        let m = pm_color::ycbcr::Matrix::of(engine).unwrap();
        let k = color::matrix_coefficients(code).unwrap().unwrap();
        for ycc in [[0.5, 0.1, -0.2], [0.9, -0.4, 0.3], [0.1, 0.0, 0.0]] {
            let (x, y) = (m.ycbcr_to_rgb(ycc), color::ycbcr_to_rgb(ycc, k));
            assert!(x.iter().zip(y).all(|(a, b)| (a - b).abs() < 1e-12), "{engine:?}");
        }
    }
    for (bits, range, full) in [(8, Range::Limited, false), (10, Range::Limited, false), (10, Range::Full, true)] {
        let q = pm_color::ycbcr::Quantizer::new(bits, range);
        for code in [0u32, 16, 64, 128, 235, 512, 940, 1023].into_iter().filter(|c| *c < 1 << bits) {
            let c = f64::from(code);
            assert!((q.luma_value(code) - color::dequantize_luma(c, u32::from(bits), full)).abs() < 1e-12);
            assert!((q.chroma_value(code) - color::dequantize_chroma(c, u32::from(bits), full)).abs() < 1e-12);
        }
    }
}
