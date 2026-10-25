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
fn css_srgb_gamut_mapping_agrees_with_independent_reference() {
    let engine = pm_color::css_srgb;
    let reference = color::css_gamut_map_srgb;

    // The mapping is relative colorimetric: every in-gamut input is exact.
    for r in grid().step_by(4) {
        for g in grid().step_by(4) {
            for b in grid().step_by(4) {
                let rgb = [r, g, b];
                assert_eq!(engine(rgb), rgb);
                assert_eq!(reference(rgb), rgb);
            }
        }
    }

    // Include the CSS Color 4 P3-yellow example and other P3 authored colors.
    // Both matrices and the extended transfer are supplied by the f64
    // reference, independently of the engine's gamut mapping implementation.
    let p3_to_srgb = |p3: [f64; 3]| {
        let xyz = color::apply(
            &Primaries::P3_D65.to_xyz(),
            p3.map(|v| Transfer::Srgb.to_linear(v)),
        );
        color::apply(&color::invert(&Primaries::BT709.to_xyz()), xyz)
            .map(|v| Transfer::Srgb.from_linear(v))
    };
    let mut samples = vec![
        p3_to_srgb([1.0, 1.0, 0.0]),
        p3_to_srgb([0.0, 1.0, 0.0]),
        p3_to_srgb([1.0, 0.0, 0.0]),
        p3_to_srgb([1.0, 0.5, 0.0]),
        p3_to_srgb([0.2, 1.0, 0.3]),
    ];
    let extended = [-0.5, -0.1, 0.0, 0.3, 0.7, 1.0, 1.1, 1.5];
    for r in extended {
        for g in extended {
            for b in extended {
                let rgb = [r, g, b];
                if rgb.iter().any(|v| !(0.0..=1.0).contains(v)) {
                    samples.push(rgb);
                }
            }
        }
    }
    let mut max_error = 0.0f64;
    let mut worst = [0.0; 3];
    for rgb in &samples {
        let (actual, expected) = (engine(*rgb), reference(*rgb));
        assert!(
            actual.iter().all(|v| (0.0..=1.0).contains(v)),
            "engine {rgb:?}: {actual:?}"
        );
        assert!(
            expected.iter().all(|v| (0.0..=1.0).contains(v)),
            "reference {rgb:?}: {expected:?}"
        );
        let error = (0..3)
            .map(|c| (actual[c] - expected[c]).abs())
            .fold(0.0f64, f64::max);
        if error > max_error {
            max_error = error;
            worst = *rgb;
        }
    }
    println!(
        "CSS sRGB gamut: {} extended samples, max f64 channel error {max_error:.9} at {worst:?}",
        samples.len()
    );
    assert!(
        max_error < 5e-4,
        "CSS gamut mapper disagreement at {worst:?}: {max_error}"
    );
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
            (pm_color::transfer::bt1886_decode(v), Transfer::Bt1886.to_linear(v), 1e-12),
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
        (pm_protocol::color::Primaries::Bt601_525, Primaries::BT601_525),
        (pm_protocol::color::Primaries::Bt601_625, Primaries::BT601_625),
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

/// BT.2087's published BT.2020 → BT.709 matrix, the engine's conversions
/// to Rec.709 for every primaries it reads, and its BT.2390 EETF.
#[test]
fn gamut_conversions_and_the_eetf_agree() {
    let to_709 = |p: Primaries| color::mul(&color::invert(&Primaries::BT709.to_xyz()), &p.to_xyz());
    let bt2087 = [
        [1.6605, -0.5876, -0.0728],
        [-0.1246, 1.1329, -0.0083],
        [-0.0182, -0.1006, 1.1187],
    ];
    let derived = to_709(Primaries::BT2020);
    for i in 0..3 {
        for j in 0..3 {
            assert!(
                (derived[i][j] - bt2087[i][j]).abs() < 1e-4,
                "BT.2087 [{i}][{j}]"
            );
        }
    }
    for (engine, reference) in [
        (
            pm_protocol::color::Primaries::Bt601_525,
            Primaries::BT601_525,
        ),
        (
            pm_protocol::color::Primaries::Bt601_625,
            Primaries::BT601_625,
        ),
        (pm_protocol::color::Primaries::DisplayP3, Primaries::P3_D65),
        (pm_protocol::color::Primaries::Bt2020, Primaries::BT2020),
    ] {
        let ours =
            pm_color::primaries::conversion(engine, pm_protocol::color::Primaries::Bt709).unwrap();
        let want = to_709(reference);
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (ours[i][j] - want[i][j]).abs() < 1e-9,
                    "{engine:?} [{i}][{j}]"
                );
            }
        }
    }
    // Identity below the knee; the source peak onto the target peak.
    let pq = |nits: f64| Transfer::Pq.from_linear(nits / 10_000.0);
    assert!((color::eetf_bt2390(pq(1000.0), 1000.0, 203.0) - pq(203.0)).abs() < 1e-12);
    assert_eq!(color::eetf_bt2390(0.3, 1000.0, 203.0), 0.3);
    for i in 0..=1000 {
        let e = f64::from(i) / 1000.0;
        let (x, y) = (
            pm_color::tonemap::eetf(e, 1000.0),
            color::eetf_bt2390(e, 1000.0, 203.0),
        );
        assert!((x - y).abs() < 1e-12, "EETF at {e}: {x} vs {y}");
    }
}

/// The engine's range normalization at the decoder depths video reaches,
/// including VideoToolbox's 16-bit ProRes layouts.
#[test]
fn range_normalization_agrees_at_12_and_16_bits() {
    use pm_protocol::color::{SampleAlignment, normalization};
    for (bits, alignment) in [(12u8, SampleAlignment::Lsb), (16, SampleAlignment::Full16)] {
        for range in [Range::Limited, Range::Full] {
            let n = normalization(bits, alignment, range);
            let full = range == Range::Full;
            for code in [0.0, 16.0, 64.0, 128.0, 235.0, 240.0, 255.0]
                .map(|c| c * f64::from(1u32 << (bits - 8)))
            {
                let y = (code - n.luma_offset) / n.luma_scale;
                let c = (code - n.chroma_offset) / n.chroma_scale;
                assert!(
                    (y - color::dequantize_luma(code, u32::from(bits), full)).abs() < 1e-12,
                    "{bits} {range:?} Y′ {code}"
                );
                assert!(
                    (c - color::dequantize_chroma(code, u32::from(bits), full)).abs() < 1e-12,
                    "{bits} {range:?} C {code}"
                );
            }
        }
    }
}
