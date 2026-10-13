//! `Math.pow` and `**` as V8 15.2 computes them: the ECMAScript special cases
//! of `v8::internal::math::pow` (`src/numbers/ieee754.cc`), then, because V8's
//! `--use-std-math-pow` defaults to true on every platform but AIX, the C
//! library's `pow` of the platform it runs on.
//!
//! Native builds therefore call the platform `pow` too (`f64::powf`), which on
//! macOS is the same libsystem_m `pow` Electron's V8 calls: bit-identical to
//! the shipping editor. wasm32 has no platform `pow`. With the `js-pow`
//! feature it imports the embedding JavaScript's `Math.pow` (`pm_expr.pow`),
//! which in the editor's renderer is V8's, so the result is again identical.
//! Without it, it falls back to [`correctly_rounded_pow`], which agrees with
//! macOS's `pow` on 99.91% of measured inputs (Apple's `pow` is off by one
//! unit in the last place on the rest). See `docs/engine/expressions.md`.

/// `Number::exponentiate` / `Math.pow` in V8 15.2.
pub(crate) fn js_pow(mut x: f64, y: f64) -> f64 {
    if y.is_nan() {
        return f64::NAN;
    }
    if y.is_infinite() && (x == 1.0 || x == -1.0) {
        return f64::NAN;
    }
    if x.is_nan() {
        x = f64::NAN;
    }
    if y == 2.0 {
        return x * x;
    } else if y == 0.5 {
        if x.is_infinite() {
            return f64::INFINITY;
        }
        return (x + 0.0).sqrt();
    }
    platform_pow(x, y)
}

#[cfg(not(target_family = "wasm"))]
fn platform_pow(x: f64, y: f64) -> f64 {
    x.powf(y)
}

#[cfg(all(target_family = "wasm", feature = "js-pow"))]
#[allow(unsafe_code)]
mod host {
    // The embedding JavaScript's `Math.pow`, imported as `pm_expr.pow`.
    #[link(wasm_import_module = "pm_expr")]
    unsafe extern "C" {
        pub(super) safe fn pow(x: f64, y: f64) -> f64;
    }
}

#[cfg(all(target_family = "wasm", feature = "js-pow"))]
fn platform_pow(x: f64, y: f64) -> f64 {
    host::pow(x, y)
}

#[cfg(all(target_family = "wasm", not(feature = "js-pow")))]
fn platform_pow(x: f64, y: f64) -> f64 {
    correctly_rounded_pow(x, y)
}

/// `pow` with every C99 Annex F special case handled here and the finite
/// positive-base case from `pxfm::f_pow`, which is correctly rounded for
/// normal results. pxfm 0.1.30 itself mishandles `pow(NaN, 0)`, negative
/// zero and infinite bases with non-integer exponents, and rounds some
/// subnormal results wrongly (`pow(2, -1075)` gives 5e-324, not 0); the first
/// three are fixed here, the last remains for results below 2^-1022.
#[cfg_attr(any(not(target_family = "wasm"), feature = "js-pow"), allow(dead_code))]
pub(crate) fn correctly_rounded_pow(x: f64, y: f64) -> f64 {
    if y == 0.0 || x == 1.0 {
        return 1.0;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if y.is_infinite() {
        let ax = x.abs();
        if ax == 1.0 {
            return 1.0;
        }
        return if (ax < 1.0) == (y < 0.0) {
            f64::INFINITY
        } else {
            0.0
        };
    }
    let y_integer = y.trunc() == y;
    let y_odd = y_integer && y.abs() < 9_007_199_254_740_992.0 && y % 2.0 != 0.0;
    if x == 0.0 {
        let negative = x.is_sign_negative() && y_odd;
        return match (y < 0.0, negative) {
            (true, true) => f64::NEG_INFINITY,
            (true, false) => f64::INFINITY,
            (false, true) => -0.0,
            (false, false) => 0.0,
        };
    }
    if x.is_infinite() {
        let negative = x < 0.0 && y_odd;
        return match (y < 0.0, negative) {
            (true, true) => -0.0,
            (true, false) => 0.0,
            (false, true) => f64::NEG_INFINITY,
            (false, false) => f64::INFINITY,
        };
    }
    if x < 0.0 {
        if !y_integer {
            return f64::NAN;
        }
        let magnitude = pxfm::f_pow(-x, y);
        return if y_odd { -magnitude } else { magnitude };
    }
    pxfm::f_pow(x, y)
}

#[cfg(test)]
mod tests {
    use super::correctly_rounded_pow;

    /// The wasm32 fallback must agree with the platform `pow` on every
    /// special result (zeros, infinities, NaN, one), whatever the rounding of
    /// ordinary results.
    #[test]
    fn special_cases_match_the_platform() {
        let values = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.5,
            -0.5,
            2.0,
            -2.0,
            3.0,
            -3.0,
            1.5,
            -1.5,
            10.0,
            0.1,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            f64::MIN_POSITIVE,
            5e-324,
            -5e-324,
            f64::MAX,
            -f64::MAX,
            1e300,
            1e-300,
            9007199254740993.0,
            -9007199254740991.0,
            1023.0,
            1024.0,
            -1074.0,
            1.0000000000000002,
            0.9999999999999999,
        ];
        let special = |v: f64| v.is_nan() || v.is_infinite() || v == 0.0 || v.abs() == 1.0;
        let mut mismatches = Vec::new();
        for &x in &values {
            for &y in &values {
                let platform = x.powf(y);
                let ours = correctly_rounded_pow(x, y);
                if !(special(platform) || special(ours)) {
                    continue;
                }
                let same =
                    platform.to_bits() == ours.to_bits() || (platform.is_nan() && ours.is_nan());
                // At x = 1 - 2^-53, Apple's pow returns 1 where the correctly
                // rounded results are 1 + 2^-52 (y = -1) and x itself (y = 0.5;
                // JavaScript takes sqrt there before calling pow).
                let known = x == 0.9999999999999999 && (y == -1.0 || y == 0.5);
                if !same && !known {
                    mismatches.push(format!(
                        "pow({x:e}, {y:e}): platform {platform:e}, fallback {ours:e}"
                    ));
                }
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    }
}
