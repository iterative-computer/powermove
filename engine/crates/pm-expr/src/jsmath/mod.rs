//! `Math` functions bit-identical to the V8 that ships in Powermove's Electron
//! (V8 15.2, Electron 44) on every Rust target, wasm32 included.
//!
//! V8 15.2 implements `Math.sin`, `cos`, `tan`, `asin`, `acos`, `atan`,
//! `atan2`, `exp`, `log`, and `pow` with LLVM libc (`src/base/ieee754.cc`
//! includes `third_party/llvm-libc/src/shared/math.h`), not fdlibm. Measured
//! against 300-digit references, its `sin`, `cos`, `tan`, `asin`, `acos`,
//! `atan2`, `exp`, and `log` are correctly rounded, so any correctly rounded
//! implementation reproduces them; these come from `pxfm` (a Rust port of the
//! CORE-MATH and LLVM libc algorithms). `atan` and `pow` are not correctly
//! rounded, so they are ported line for line from the LLVM libc revision
//! Chromium 152 pins ([`atan`](mod@atan), [`pow`](mod@pow)). The fdlibm ports in
//! the `libm` crate differ from V8 15.2 in about 1 in 30 `sin` results, which
//! is why they are not used. See `docs/engine/expressions.md`.
//!
//! The remaining functions are exact operations with ECMAScript edge cases.

mod atan;
mod dd;
mod hexf;
mod pow;
mod tables;

pub fn sin(x: f64) -> f64 {
    pxfm::f_sin(x)
}

pub fn cos(x: f64) -> f64 {
    pxfm::f_cos(x)
}

pub fn tan(x: f64) -> f64 {
    pxfm::f_tan(x)
}

pub fn asin(x: f64) -> f64 {
    pxfm::f_asin(x)
}

pub fn acos(x: f64) -> f64 {
    pxfm::f_acos(x)
}

pub fn atan(x: f64) -> f64 {
    atan::atan(x)
}

/// `Math.atan2(y, x)`.
pub fn atan2(y: f64, x: f64) -> f64 {
    pxfm::f_atan2(y, x)
}

pub fn exp(x: f64) -> f64 {
    pxfm::f_exp(x)
}

pub fn log(x: f64) -> f64 {
    pxfm::f_log(x)
}

/// `Math.pow(x, y)` and `x ** y`.
pub fn pow(x: f64, y: f64) -> f64 {
    pow::js_pow(x, y)
}

pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

pub fn abs(x: f64) -> f64 {
    x.abs()
}

pub fn floor(x: f64) -> f64 {
    x.floor()
}

pub fn ceil(x: f64) -> f64 {
    x.ceil()
}

/// `Math.round`: the nearest integer, ties toward +∞, keeping -0 for
/// -0.5 <= x <= -0.
pub fn round(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    if (-0.5..0.0).contains(&x) {
        return -0.0;
    }
    let r = x.ceil();
    if r - 0.5 > x { r - 1.0 } else { r }
}

/// `Math.sign`.
pub fn sign(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 {
        x
    } else if x > 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// `Math.max(...values)`: NaN wins, +0 beats -0, and no arguments give -∞.
pub fn max(values: &[f64]) -> f64 {
    let mut out = f64::NEG_INFINITY;
    for &v in values {
        if v.is_nan() || out.is_nan() {
            out = f64::NAN;
        } else if v > out || (v == 0.0 && out == 0.0 && out.is_sign_negative()) {
            out = v;
        }
    }
    out
}

/// `Math.min(...values)`: NaN wins, -0 beats +0, and no arguments give +∞.
pub fn min(values: &[f64]) -> f64 {
    let mut out = f64::INFINITY;
    for &v in values {
        if v.is_nan() || out.is_nan() {
            out = f64::NAN;
        } else if v < out || (v == 0.0 && out == 0.0 && v.is_sign_negative()) {
            out = v;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_matches_ecmascript() {
        assert_eq!(round(0.49999999999999994), 0.0);
        assert_eq!(round(0.5), 1.0);
        assert_eq!(round(-0.5).to_bits(), (-0.0f64).to_bits());
        assert_eq!(round(-0.2).to_bits(), (-0.0f64).to_bits());
        assert_eq!(round(-1.5), -1.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(4503599627370495.5), 4503599627370496.0);
        assert_eq!(round(-4503599627370495.5), -4503599627370495.0);
        assert_eq!(round(9007199254740993.0), 9007199254740992.0);
    }

    #[test]
    fn max_min_edge_cases() {
        assert_eq!(max(&[]), f64::NEG_INFINITY);
        assert_eq!(min(&[]), f64::INFINITY);
        assert!(max(&[1.0, f64::NAN, 3.0]).is_nan());
        assert_eq!(max(&[-0.0, 0.0]).to_bits(), 0);
        assert_eq!(min(&[0.0, -0.0]).to_bits(), (-0.0f64).to_bits());
    }

    #[test]
    fn pow_ecmascript_cases() {
        assert!(pow(1.0, f64::INFINITY).is_nan());
        assert!(pow(-1.0, f64::NEG_INFINITY).is_nan());
        assert!(pow(1.0, f64::NAN).is_nan());
        assert_eq!(pow(f64::NAN, 0.0), 1.0);
        assert_eq!(pow(-0.0, 0.5).to_bits(), 0);
        assert_eq!(pow(f64::NEG_INFINITY, 0.5), f64::INFINITY);
        assert_eq!(pow(-8.0, 1.0 / 3.0).is_nan(), true);
        assert_eq!(pow(2.0, 10.0), 1024.0);
        assert_eq!(pow(-2.0, 3.0), -8.0);
        assert_eq!(pow(0.0, -1.0), f64::INFINITY);
        assert_eq!(pow(-0.0, -1.0), f64::NEG_INFINITY);
        assert_eq!(pow(-0.0, -2.0), f64::INFINITY);
    }

    #[test]
    fn atan_tiny_inputs_follow_llvm() {
        // |x| < 2^-26 returns fma(-2^-54, x, x), which is x - ulp/2-ish, not x.
        let x = 1.4604091644287108e-8;
        assert_eq!(atan(x).to_bits(), 0x3e4f5caf2d7f950b);
        assert_eq!(atan(-0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(atan(f64::INFINITY), core::f64::consts::FRAC_PI_2);
    }
}
