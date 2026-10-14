//! Transfer functions (section 9, "The rule" and "Per-source interpretation").
//!
//! E is the sRGB piecewise curve extended to all reals: odd-symmetric below
//! 0 and the power segment continued above 1, so superwhites and sub-blacks
//! survive every transform (clipping happens only at output). The f64
//! functions are contract 4's normative reference; the f32 functions are the
//! same formulas evaluated the way the shaders evaluate them (see
//! [`crate::wgsl`]), for CPU paths that must agree with the GPU.

use pm_protocol::color::{Transfer, WorkingEncoding};

pub use pm_protocol::color::{bt709_inverse_oetf, bt1886_decode, display_decode, display_encode};

/// Linear-light break of E (below it the curve is linear).
pub const E_LINEAR_BREAK: f64 = 0.003_130_8;
/// Encoded break of E.
pub const E_ENCODED_BREAK: f64 = 0.040_45;

/// E in f32, the formula the WGSL `pm_e_encode` evaluates.
pub fn display_encode_f32(linear: f32) -> f32 {
    let m = linear.abs();
    let e = if m <= E_LINEAR_BREAK as f32 {
        12.92 * m
    } else {
        1.055 * libm::powf(m, 1.0 / 2.4) - 0.055
    };
    e.copysign(linear)
}

/// E⁻¹ in f32, the formula the WGSL `pm_e_decode` evaluates.
pub fn display_decode_f32(encoded: f32) -> f32 {
    let m = encoded.abs();
    let l = if m <= E_ENCODED_BREAK as f32 {
        m / 12.92
    } else {
        libm::powf((m + 0.055) / 1.055, 2.4)
    };
    l.copysign(encoded)
}

/// BT.709 OETF, extended odd-symmetric (the inverse of
/// [`bt709_inverse_oetf`]).
pub fn bt709_oetf(linear: f64) -> f64 {
    let m = linear.abs();
    let e = if m < 0.018 {
        4.5 * m
    } else {
        1.099 * libm::pow(m, 0.45) - 0.099
    };
    e.copysign(linear)
}

/// BT.1886 inverse EOTF with zero black level (gamma 1/2.4), extended.
pub fn bt1886_encode(linear: f64) -> f64 {
    libm::pow(linear.abs(), 1.0 / 2.4).copysign(linear)
}

/// Pure gamma 2.2, extended odd-symmetric.
pub fn gamma22_decode(encoded: f64) -> f64 {
    libm::pow(encoded.abs(), 2.2).copysign(encoded)
}

/// Inverse of [`gamma22_decode`].
pub fn gamma22_encode(linear: f64) -> f64 {
    libm::pow(linear.abs(), 1.0 / 2.2).copysign(linear)
}

/// Why a transfer cannot be read into a Standard project yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferError {
    /// PQ and HLG sources need the BT.2446 Method A tone mapping (Phase 3b,
    /// contract 4 open question 1); they are not read here.
    NeedsToneMapping(Transfer),
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransferError::NeedsToneMapping(t) => {
                write!(f, "{t:?} sources need HDR tone mapping (Phase 3b)")
            }
        }
    }
}

impl std::error::Error for TransferError {}

/// Encoded value under `transfer` to linear light.
pub fn to_linear(transfer: Transfer, encoded: f64) -> Result<f64, TransferError> {
    Ok(match transfer {
        Transfer::DisplayE => display_decode(encoded),
        Transfer::Bt1886 => bt1886_decode(encoded),
        Transfer::SceneReferredBt709 => bt709_inverse_oetf(encoded),
        Transfer::Linear => encoded,
        Transfer::Gamma22 => gamma22_decode(encoded),
        Transfer::Pq | Transfer::Hlg => return Err(TransferError::NeedsToneMapping(transfer)),
    })
}

/// Linear light to an encoded value under `transfer`.
pub fn from_linear(transfer: Transfer, linear: f64) -> Result<f64, TransferError> {
    Ok(match transfer {
        Transfer::DisplayE => display_encode(linear),
        Transfer::Bt1886 => bt1886_encode(linear),
        Transfer::SceneReferredBt709 => bt709_oetf(linear),
        Transfer::Linear => linear,
        Transfer::Gamma22 => gamma22_encode(linear),
        Transfer::Pq | Transfer::Hlg => return Err(TransferError::NeedsToneMapping(transfer)),
    })
}

/// A source value (Rec.709 primaries) under `transfer` into the working
/// encoding. In Standard an E source is the identity: code values pass
/// through unchanged (section 9's rule).
pub fn to_working(
    transfer: Transfer,
    working: WorkingEncoding,
    encoded: f64,
) -> Result<f64, TransferError> {
    match (working, transfer) {
        (WorkingEncoding::Standard, Transfer::DisplayE) => Ok(encoded),
        (WorkingEncoding::Standard, _) => Ok(display_encode(to_linear(transfer, encoded)?)),
        (WorkingEncoding::LinearLight, _) => to_linear(transfer, encoded),
    }
}

/// A working value into an output encoded with E (every Standard output).
pub fn working_to_display(working: WorkingEncoding, value: f64) -> f64 {
    match working {
        WorkingEncoding::Standard => value,
        WorkingEncoding::LinearLight => display_encode(value),
    }
}

/// The largest code of an n-bit unsigned value.
pub const fn code_max(bits: u8) -> u32 {
    if bits >= 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    }
}

/// A full-range code to its unit value, exactly `code / (2ⁿ − 1)`.
pub fn code_to_unit(code: u32, bits: u8) -> f64 {
    code as f64 / code_max(bits) as f64
}

/// A unit value to a full-range code: clipped to 0..1 (clipping happens only
/// at output), then rounded half up as the section 2 bars state.
pub fn unit_to_code(value: f64, bits: u8) -> u32 {
    let max = code_max(bits) as f64;
    let v = if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    };
    libm::floor(v * max + 0.5) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f32_e_agrees_with_the_reference() {
        // The f32 formula stays within a few ULP of the f64 reference over
        // the extended domain, and round trips every 16-bit code exactly.
        let mut worst = 0.0f64;
        let mut x = -2.0f64;
        while x <= 4.0 {
            let r = display_encode(x);
            let f = display_encode_f32(x as f32) as f64;
            worst = worst.max((f - r).abs() / r.abs().max(1e-6));
            x += 0.000_37;
        }
        assert!(worst < 2e-6, "relative error {worst}");
        for code in 0..=65535u32 {
            let v = code as f32 / 65535.0;
            let back = display_encode_f32(display_decode_f32(v));
            assert_eq!((back * 65535.0).round() as u32, code, "code {code}");
        }
    }

    #[test]
    fn standard_reads_e_sources_as_the_identity() {
        for v in [-0.25, 0.0, 0.003, 0.5, 1.0, 1.7] {
            assert_eq!(
                to_working(Transfer::DisplayE, WorkingEncoding::Standard, v).unwrap(),
                v
            );
        }
    }

    #[test]
    fn interpret_alternatives_round_trip() {
        for t in [
            Transfer::DisplayE,
            Transfer::Bt1886,
            Transfer::SceneReferredBt709,
            Transfer::Linear,
            Transfer::Gamma22,
        ] {
            let mut x = -1.5;
            while x <= 3.0 {
                let l = to_linear(t, x).unwrap();
                let back = from_linear(t, l).unwrap();
                assert!((back - x).abs() < 1e-12 * x.abs().max(1.0), "{t:?} {x}");
                x += 0.013;
            }
        }
        assert_eq!(
            to_linear(Transfer::Pq, 0.5),
            Err(TransferError::NeedsToneMapping(Transfer::Pq))
        );
    }

    #[test]
    fn bt1886_reading_darkens_mid_tones_against_e() {
        // "Rec.709 (broadcast)" looks darker on a BT.1886 monitor, by design:
        // the same code is less light under gamma 2.4 than under E.
        let code = 0.5;
        assert!(to_linear(Transfer::Bt1886, code).unwrap() < display_decode(code));
        let w = to_working(Transfer::Bt1886, WorkingEncoding::Standard, code).unwrap();
        assert!(w < code);
    }

    #[test]
    fn codes_round_half_up_and_clip() {
        assert_eq!(unit_to_code(0.735_357, 8), 188);
        assert_eq!(unit_to_code(187.5 / 255.0, 8), 188);
        assert_eq!(unit_to_code(-0.2, 8), 0);
        assert_eq!(unit_to_code(1.3, 10), 1023);
        assert_eq!(unit_to_code(f64::NAN, 8), 0);
        for code in 0..=255 {
            assert_eq!(unit_to_code(code_to_unit(code, 8), 8), code);
        }
        for code in 0..=65535 {
            assert_eq!(unit_to_code(code_to_unit(code, 16), 16), code);
        }
    }
}
