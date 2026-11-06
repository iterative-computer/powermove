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

#[cfg(test)]
mod tests {
    use super::*;
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
                metadata: None
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
