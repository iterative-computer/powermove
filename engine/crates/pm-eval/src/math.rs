//! Transcendental functions for evaluation (plan section 7, "Determinism").
//!
//! Every target uses the `libm` crate, so native and WASM answers are bit
//! identical (S5 measured identical SHA-256 over 2.7 million values, and 7,816
//! differences with platform math as the negative control). Nothing in
//! `pm-eval`, or in a crate that evaluates on its behalf (`pm-expr`), may call
//! `f64::sin` and friends: they lower to the platform's math library.
//!
//! The operations IEEE 754 requires to be correctly rounded (`+ - * /`,
//! `sqrt`, `floor`, `ceil`, `trunc`, `abs`) are deterministic everywhere and
//! use Rust's own. `fma` is never used, so expressions keep JavaScript's
//! operation order (the one exception reproduces V8's own fused
//! multiply-add in `parseInt`, `js_sem::parse_int`).
//!
//! These do not always equal V8's results to the bit: V8 uses its own fdlibm
//! port for `sin`, `cos`, `tan` (S5 measured world matrices within 2 ulps,
//! 32 ulps where `tan(89.9°)` amplifies, far inside the section 25 tolerance).

#[inline]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

#[inline]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

#[inline]
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}

#[inline]
pub fn asin(x: f64) -> f64 {
    libm::asin(x)
}

#[inline]
pub fn acos(x: f64) -> f64 {
    libm::acos(x)
}

#[inline]
pub fn atan(x: f64) -> f64 {
    libm::atan(x)
}

#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

#[inline]
pub fn sinh(x: f64) -> f64 {
    libm::sinh(x)
}

#[inline]
pub fn cosh(x: f64) -> f64 {
    libm::cosh(x)
}

#[inline]
pub fn tanh(x: f64) -> f64 {
    libm::tanh(x)
}

#[inline]
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

#[inline]
pub fn expm1(x: f64) -> f64 {
    libm::expm1(x)
}

#[inline]
pub fn log(x: f64) -> f64 {
    libm::log(x)
}

#[inline]
pub fn log1p(x: f64) -> f64 {
    libm::log1p(x)
}

#[inline]
pub fn log2(x: f64) -> f64 {
    libm::log2(x)
}

#[inline]
pub fn log10(x: f64) -> f64 {
    libm::log10(x)
}

#[inline]
pub fn cbrt(x: f64) -> f64 {
    libm::cbrt(x)
}

#[inline]
pub fn hypot(x: f64, y: f64) -> f64 {
    libm::hypot(x, y)
}

/// `Math.pow(x, y)` and `x ** y`, with ECMAScript's special cases where they
/// differ from C's `pow`: a NaN exponent always gives NaN, and `(±1) ** ±∞`
/// is NaN (C answers 1).
#[inline]
pub fn pow(x: f64, y: f64) -> f64 {
    if y.is_nan() {
        return f64::NAN;
    }
    if y.is_infinite() && x.abs() == 1.0 {
        return f64::NAN;
    }
    libm::pow(x, y)
}

/// `Math.sqrt` (correctly rounded on every target).
#[inline]
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

/// Degrees to radians the way the legacy code writes it: `deg * Math.PI / 180`
/// (two roundings, in that order).
#[inline]
pub fn deg_to_rad(deg: f64) -> f64 {
    deg * core::f64::consts::PI / 180.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pow_follows_ecmascript() {
        assert!(pow(1.0, f64::NAN).is_nan());
        assert!(pow(1.0, f64::INFINITY).is_nan());
        assert!(pow(-1.0, f64::NEG_INFINITY).is_nan());
        assert_eq!(pow(f64::NAN, 0.0), 1.0);
        assert_eq!(pow(2.0, 10.0), 1024.0);
    }
}
