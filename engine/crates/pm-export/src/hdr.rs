//! Explicit BT.2100 delivery from the engine's Rec.709 working space.
//!
//! PQ uses ST 2084 absolute luminance; HLG uses BT.2100's reference-display
//! OOTF (including its luminance-dependent system gamma). No display headroom
//! or window state enters an export. Mastering white/peak are caller intent.

use crate::convert::{ALPHA_GUARD, e_decode};
use crate::{AlphaOutput, ColorTags, Error, WorkingEncoding};

pub fn validate(tags: ColorTags) -> Result<(), Error> {
    let valid = match tags {
        ColorTags::Rec2100Pq { white_nits, .. } => (1..=10_000).contains(&white_nits),
        ColorTags::Rec2100Hlg {
            white_nits,
            peak_nits,
            ..
        } => (400..=4000).contains(&peak_nits) && white_nits > 0 && white_nits <= peak_nits,
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidSpec(
            "invalid HDR diffuse white or HLG mastering peak".into(),
        ))
    }
}

/// ST 2084 OETF, input in cd/m². Out-of-domain light clips at mastering limits.
pub fn pq(nits: f64) -> f64 {
    let p = (nits.clamp(0.0, 10_000.0) / 10_000.0).powf(2610.0 / 16384.0);
    ((3424.0 / 4096.0 + 2413.0 / 128.0 * p) / (1.0 + 2392.0 / 128.0 * p)).powf(2523.0 / 32.0)
}

/// BT.2100 HLG scene OETF.
pub fn hlg(scene: f64) -> f64 {
    let x = scene.clamp(0.0, 1.0);
    if x <= 1.0 / 12.0 {
        (3.0 * x).sqrt()
    } else {
        0.178_832_77 * (12.0 * x - 0.284_668_92).ln() + 0.559_910_73
    }
}

pub(crate) fn output_ycbcra(
    w: [f32; 4],
    working: WorkingEncoding,
    alpha: AlphaOutput,
    tags: ColorTags,
) -> ([f64; 3], f64) {
    let a = f64::from(w[3]).clamp(0.0, 1.0);
    // Unassociate before a nonlinear transform. Opaque output composites in
    // the working domain; associated HDR output associates in the coded domain.
    let c = [w[0], w[1], w[2]].map(|v| {
        let v = if alpha == AlphaOutput::None {
            f64::from(v)
        } else if a < ALPHA_GUARD {
            0.0
        } else {
            f64::from(v) / a
        };
        match working {
            WorkingEncoding::Standard => e_decode(v),
            WorkingEncoding::Linear => v,
        }
    });
    // Linear BT.709 → BT.2020, D65. Matrix from the two RGB-to-XYZ bases.
    let c = [
        0.627_403_896 * c[0] + 0.329_283_038 * c[1] + 0.043_313_066 * c[2],
        0.069_097_289 * c[0] + 0.919_540_395 * c[1] + 0.011_362_316 * c[2],
        0.016_391_439 * c[0] + 0.088_013_308 * c[1] + 0.895_595_253 * c[2],
    ];
    let mut encoded = match tags {
        ColorTags::Rec2100Pq { white_nits, .. } => c.map(|v| pq(v * f64::from(white_nits))),
        ColorTags::Rec2100Hlg {
            white_nits,
            peak_nits,
            ..
        } => {
            let gamma = 1.2 + 0.42 * (f64::from(peak_nits) / 1000.0).log10();
            let display = c.map(|v| (v * f64::from(white_nits) / f64::from(peak_nits)).max(0.0));
            let yd = 0.2627 * display[0] + 0.6780 * display[1] + 0.0593 * display[2];
            let factor = if yd > 0.0 {
                yd.powf((1.0 - gamma) / gamma)
            } else {
                0.0
            };
            display.map(|v| hlg(v * factor))
        }
        _ => unreachable!("HDR tags checked by caller"),
    };
    if alpha == AlphaOutput::Premultiplied {
        encoded = encoded.map(|v| v * a);
    }
    let y = 0.2627 * encoded[0] + 0.6780 * encoded[1] + 0.0593 * encoded[2];
    (
        [
            y,
            (encoded[2] - y) / (2.0 * (1.0 - 0.0593)),
            (encoded[0] - y) / (2.0 * (1.0 - 0.2627)),
        ],
        if alpha == AlphaOutput::None { 1.0 } else { a },
    )
}

/// The transfer functions over many values at once: Apple's vectorised
/// `pow` and `log` (vForce) where they exist, `f64::powf` and `ln`
/// elsewhere. Both are accurate to an ulp; the codes a frame quantizes to
/// are the scalar path's (`a_row_is_the_scalar_paths_codes`).
mod batch {
    #[cfg(target_os = "macos")]
    #[link(name = "Accelerate", kind = "framework")]
    unsafe extern "C" {
        fn vvpow(z: *mut f64, y: *const f64, x: *const f64, n: *const i32);
        fn vvlog(y: *mut f64, x: *const f64, n: *const i32);
    }

    /// `values[i] = values[i].powf(exponent)`.
    pub fn pow(values: &mut [f64], exponent: f64, scratch: &mut Vec<f64>) {
        #[cfg(target_os = "macos")]
        if let Ok(n) = i32::try_from(values.len()) {
            scratch.clear();
            scratch.resize(values.len(), exponent);
            // SAFETY: both buffers hold `n` values; vvpow reads x and y and
            // writes z, which may alias x.
            unsafe { vvpow(values.as_mut_ptr(), scratch.as_ptr(), values.as_ptr(), &n) };
            return;
        }
        let _ = scratch;
        values.iter_mut().for_each(|v| *v = v.powf(exponent));
    }

    /// `values[i] = values[i].ln()`.
    pub fn ln(values: &mut [f64]) {
        #[cfg(target_os = "macos")]
        if let Ok(n) = i32::try_from(values.len()) {
            // SAFETY: the buffer holds `n` values; vvlog may write in place.
            unsafe { vvlog(values.as_mut_ptr(), values.as_ptr(), &n) };
            return;
        }
        values.iter_mut().for_each(|v| *v = v.ln());
    }
}

/// Buffers a row of the HDR transform reuses.
#[derive(Default)]
pub(crate) struct RowScratch {
    a: Vec<f64>,
    c: Vec<f64>,
    t: Vec<f64>,
    exp: Vec<f64>,
}

/// [`output_ycbcra`] of a row of working RGBA pixels, each transfer function
/// evaluated over the whole row at once: the same formulas in the same
/// order, written to `out`.
pub(crate) fn output_row(
    row: &[f32],
    working: WorkingEncoding,
    alpha: AlphaOutput,
    tags: ColorTags,
    scratch: &mut RowScratch,
    out: &mut [([f64; 3], f64)],
) {
    let w = row.len() / 4;
    let RowScratch { a, c, t, exp } = scratch;
    a.clear();
    c.clear();
    // Alpha, and the unassociated working values, channel-major.
    a.extend(row.chunks_exact(4).map(|p| f64::from(p[3]).clamp(0.0, 1.0)));
    c.resize(3 * w, 0.0);
    for (i, p) in row.chunks_exact(4).enumerate() {
        for k in 0..3 {
            c[k * w + i] = if alpha == AlphaOutput::None {
                f64::from(p[k])
            } else if a[i] < ALPHA_GUARD {
                0.0
            } else {
                f64::from(p[k]) / a[i]
            };
        }
    }
    if working == WorkingEncoding::Standard {
        // E⁻¹ (`e_decode`): the curve's power over the whole row.
        t.clear();
        t.extend(c.iter().map(|v| {
            let m = v.abs();
            if m <= 0.040_45 {
                1.0
            } else {
                (m + 0.055) / 1.055
            }
        }));
        batch::pow(t, 2.4, exp);
        for (v, p) in c.iter_mut().zip(t.iter()) {
            let m = v.abs();
            let decoded = if m <= 0.040_45 { m / 12.92 } else { *p };
            *v = decoded.copysign(*v);
        }
    }
    // Linear BT.709 → BT.2020, D65.
    for i in 0..w {
        let (r, g, b) = (c[i], c[w + i], c[2 * w + i]);
        c[i] = 0.627_403_896 * r + 0.329_283_038 * g + 0.043_313_066 * b;
        c[w + i] = 0.069_097_289 * r + 0.919_540_395 * g + 0.011_362_316 * b;
        c[2 * w + i] = 0.016_391_439 * r + 0.088_013_308 * g + 0.895_595_253 * b;
    }
    match tags {
        ColorTags::Rec2100Pq { white_nits, .. } => {
            // `pq`, over the row.
            let white = f64::from(white_nits);
            c.iter_mut()
                .for_each(|v| *v = (*v * white).clamp(0.0, 10_000.0) / 10_000.0);
            batch::pow(c, 2610.0 / 16384.0, exp);
            c.iter_mut().for_each(|p| {
                *p = (3424.0 / 4096.0 + 2413.0 / 128.0 * *p) / (1.0 + 2392.0 / 128.0 * *p)
            });
            batch::pow(c, 2523.0 / 32.0, exp);
        }
        ColorTags::Rec2100Hlg {
            white_nits,
            peak_nits,
            ..
        } => {
            let gamma = 1.2 + 0.42 * (f64::from(peak_nits) / 1000.0).log10();
            let (white, peak) = (f64::from(white_nits), f64::from(peak_nits));
            c.iter_mut().for_each(|v| *v = (*v * white / peak).max(0.0));
            // The system gamma's factor, `yd^((1 − γ)/γ)`, per pixel.
            t.clear();
            t.extend((0..w).map(|i| {
                let yd = 0.2627 * c[i] + 0.6780 * c[w + i] + 0.0593 * c[2 * w + i];
                if yd > 0.0 { yd } else { 1.0 }
            }));
            let positive: Vec<bool> = (0..w)
                .map(|i| 0.2627 * c[i] + 0.6780 * c[w + i] + 0.0593 * c[2 * w + i] > 0.0)
                .collect();
            batch::pow(t, (1.0 - gamma) / gamma, exp);
            for i in 0..w {
                let factor = if positive[i] { t[i] } else { 0.0 };
                for k in 0..3 {
                    c[k * w + i] = (c[k * w + i] * factor).clamp(0.0, 1.0);
                }
            }
            // `hlg`: the log segment over the row.
            t.clear();
            t.extend(c.iter().map(|x| {
                if *x <= 1.0 / 12.0 {
                    1.0
                } else {
                    12.0 * x - 0.284_668_92
                }
            }));
            batch::ln(t);
            for (x, l) in c.iter_mut().zip(t.iter()) {
                *x = if *x <= 1.0 / 12.0 {
                    (3.0 * *x).sqrt()
                } else {
                    0.178_832_77 * l + 0.559_910_73
                };
            }
        }
        _ => unreachable!("HDR tags checked by caller"),
    }
    for (i, o) in out.iter_mut().enumerate().take(w) {
        let mut e = [c[i], c[w + i], c[2 * w + i]];
        if alpha == AlphaOutput::Premultiplied {
            e = e.map(|v| v * a[i]);
        }
        let y = 0.2627 * e[0] + 0.6780 * e[1] + 0.0593 * e[2];
        *o = (
            [
                y,
                (e[2] - y) / (2.0 * (1.0 - 0.0593)),
                (e[0] - y) / (2.0 * (1.0 - 0.2627)),
            ],
            if alpha == AlphaOutput::None {
                1.0
            } else {
                a[i]
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The row path gives the scalar path's values to within a part in a
    /// trillion, far below a 12-bit code (1/3504 of luma), on bright,
    /// dark, negative, out-of-range and transparent pixels.
    #[test]
    fn a_row_is_the_scalar_paths_codes() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32
        };
        let row: Vec<f32> = (0..4096)
            .flat_map(|i| {
                let scale = [0.001, 0.04, 1.0, 6.0][i % 4];
                let a = if i % 7 == 0 { next() } else { 1.0 };
                [
                    scale * (2.2 * next() - 0.1) * a,
                    scale * next() * a,
                    scale * (next() - 0.02) * a,
                    a,
                ]
            })
            .collect();
        let tagss = [
            ColorTags::Rec2100Pq {
                white_nits: 203,
                metadata: None,
            },
            ColorTags::Rec2100Hlg {
                white_nits: 203,
                peak_nits: 1000,
                metadata: None,
                dolby_vision: true,
            },
        ];
        for tags in tagss {
            for working in [WorkingEncoding::Standard, WorkingEncoding::Linear] {
                for alpha in [
                    AlphaOutput::None,
                    AlphaOutput::Straight,
                    AlphaOutput::Premultiplied,
                ] {
                    let mut out = vec![([0.0; 3], 0.0); 4096];
                    output_row(
                        &row,
                        working,
                        alpha,
                        tags,
                        &mut RowScratch::default(),
                        &mut out,
                    );
                    let mut worst = 0.0f64;
                    let mut codes = 0;
                    for (i, p) in row.chunks_exact(4).enumerate() {
                        let want = output_ycbcra([p[0], p[1], p[2], p[3]], working, alpha, tags);
                        assert_eq!(out[i].1, want.1);
                        for k in 0..3 {
                            worst = worst.max((out[i].0[k] - want.0[k]).abs());
                            let q = |v: f64| (v * 56064.0).round_ties_even();
                            codes += usize::from(q(out[i].0[k]) != q(want.0[k]));
                        }
                    }
                    assert!(worst < 1e-12, "{tags:?} {working:?} {alpha:?}: {worst:e}");
                    assert_eq!(codes, 0, "{tags:?} {working:?} {alpha:?}");
                }
            }
        }
    }

    #[test]
    fn published_neutral_signal_values() {
        assert!((pq(100.0) - 0.5080784215).abs() < 1e-9);
        assert!((pq(1000.0) - 0.7518270962).abs() < 1e-9);
        assert!((pq(10_000.0) - 1.0).abs() < 1e-12);
        assert!((hlg(1.0 / 12.0) - 0.5).abs() < 1e-12);
        assert!((hlg(1.0) - 1.0).abs() < 1e-7);
        let (c, _) = output_ycbcra(
            [1.0; 4],
            WorkingEncoding::Linear,
            AlphaOutput::None,
            ColorTags::Rec2100Hlg {
                white_nits: 203,
                peak_nits: 1000,
                metadata: None,
                dolby_vision: true,
            },
        );
        assert!((c[0] - 0.75).abs() < 0.001);
        assert!(c[1].abs() < 1e-9 && c[2].abs() < 1e-9);
    }
    #[test]
    fn invalid_mastering_is_rejected() {
        assert!(
            validate(ColorTags::Rec2100Pq {
                white_nits: 0,
                metadata: None
            })
            .is_err()
        );
        assert!(
            validate(ColorTags::Rec2100Hlg {
                white_nits: 203,
                peak_nits: 200,
                metadata: None,
                dolby_vision: true,
            })
            .is_err()
        );
    }
    #[test]
    fn hdr_metadata_payloads_are_big_endian_green_first() {
        let metadata = crate::settings::HdrMetadata {
            primaries: [[13250, 34500], [7500, 3000], [34000, 16000]],
            white: [15635, 16450],
            max_luminance: 10_000_000,
            min_luminance: 50,
            max_cll: 1000,
            max_fall: 400,
        };
        let mdcv = metadata.mdcv();
        assert_eq!(mdcv[0..2], 13250u16.to_be_bytes());
        assert_eq!(mdcv[10..12], 16000u16.to_be_bytes());
        assert_eq!(mdcv[12..14], 15635u16.to_be_bytes());
        assert_eq!(mdcv[16..20], 10_000_000u32.to_be_bytes());
        assert_eq!(mdcv[20..24], 50u32.to_be_bytes());
        assert_eq!(metadata.clli(), [0x03, 0xE8, 0x01, 0x90]);
    }
}
