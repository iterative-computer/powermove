//! JavaScript numeric semantics the legacy geometry depends on.
//!
//! The ports in this crate reproduce `Math.min`, `Math.max`, `Math.round`,
//! `Math.hypot`, and `Number(x) || 0` exactly, including NaN propagation and
//! signed zeros, because the legacy code's edge cases (for example a NaN copy
//! count drawing nothing) follow from them. Trigonometry goes through `libm`
//! on every target so native and wasm32 builds agree bit for bit (section 7,
//! determinism); `libm` and V8 both derive from fdlibm.

use core::f64::consts::PI;

/// `Math.min(a, b)`: NaN if either is NaN; `-0` is less than `+0`.
#[inline]
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        // Distinguish signed zeros like V8.
        if a.is_sign_negative() { a } else { b }
    } else if a < b {
        a
    } else {
        b
    }
}

/// `Math.max(a, b)`: NaN if either is NaN; `+0` is greater than `-0`.
#[inline]
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == b {
        if a.is_sign_positive() { a } else { b }
    } else if a > b {
        a
    } else {
        b
    }
}

/// `Math.round`: half toward +∞, preserving `-0` (V8's `Float64Round`).
#[inline]
pub fn round(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let up = x.ceil();
    if up - 0.5 > x { up - 1.0 } else { up }
}

/// `Math.hypot(a, b)` exactly as V8 computes it (scaled, Kahan-compensated
/// sum), so results match the editor bit for bit.
pub fn hypot(a: f64, b: f64) -> f64 {
    let (a, b) = (a.abs(), b.abs());
    if a.is_infinite() || b.is_infinite() {
        return f64::INFINITY;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let max = if a > b { a } else { b };
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut compensation = 0.0;
    for value in [a, b] {
        let n = value / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    libm::sqrt(sum) * max
}

/// `Number(x) || 0` for a value that is already a number: NaN and `-0` become `+0`.
#[inline]
pub fn or0(x: f64) -> f64 {
    if x.is_nan() || x == 0.0 { 0.0 } else { x }
}

/// JavaScript truthiness of a number: false for 0, -0, and NaN.
#[inline]
pub fn truthy(x: f64) -> bool {
    !(x.is_nan() || x == 0.0)
}

/// `x || fallback` for a number.
#[inline]
pub fn or(x: f64, fallback: f64) -> f64 {
    if x.is_nan() || x == 0.0 { fallback } else { x }
}

/// `degrees * Math.PI / 180`, evaluated left to right as the legacy code does.
#[inline]
pub fn radians(degrees: f64) -> f64 {
    degrees * PI / 180.0
}

/// `Math.cos`.
#[inline]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

/// `Math.sin`.
#[inline]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

/// `Math.tan`.
#[inline]
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}

/// `PM.clamp(v, a, b)` from `core/util.ts`: `v < a ? a : v > b ? b : v`.
#[inline]
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// `String(x)` for a number (ECMAScript `Number::toString`).
pub fn number_to_string(x: f64) -> String {
    if x == 0.0 {
        // `String(-0)` is "0".
        return "0".to_owned();
    }
    ryu_js::Buffer::new().format(x).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_matches_math_round() {
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -2.0);
        assert_eq!(round(0.49999999999999994), 0.0);
        assert!(round(-0.3).is_sign_negative());
        assert_eq!(round(-0.5), 0.0);
        assert!(round(-0.5).is_sign_negative());
        assert_eq!(round(4503599627370495.5), 4503599627370496.0);
        assert!(round(f64::NAN).is_nan());
    }

    #[test]
    fn min_max_propagate_nan_and_order_zeros() {
        assert!(min(1.0, f64::NAN).is_nan());
        assert!(max(f64::NAN, 1.0).is_nan());
        assert!(min(0.0, -0.0).is_sign_negative());
        assert!(max(-0.0, 0.0).is_sign_positive());
        assert_eq!(min(256.0, max(1.0, 3.0)), 3.0);
    }

    #[test]
    fn hypot_matches_v8_values() {
        assert_eq!(hypot(3.0, 4.0), 5.0);
        assert_eq!(hypot(0.0, -0.0), 0.0);
        assert_eq!(hypot(f64::NAN, f64::INFINITY), f64::INFINITY);
        assert!(hypot(f64::NAN, 1.0).is_nan());
    }

    #[test]
    fn number_strings_match_javascript() {
        assert_eq!(number_to_string(-0.0), "0");
        assert_eq!(number_to_string(12.0), "12");
        assert_eq!(number_to_string(0.1234), "0.1234");
        assert_eq!(number_to_string(-3.5e-5), "-0.000035");
        assert_eq!(number_to_string(1e-7), "1e-7");
        assert_eq!(number_to_string(1e21), "1e+21");
        assert_eq!(number_to_string(f64::NAN), "NaN");
        assert_eq!(number_to_string(f64::NEG_INFINITY), "-Infinity");
    }
}
