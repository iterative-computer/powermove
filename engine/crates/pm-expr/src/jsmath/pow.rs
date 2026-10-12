//! `Math.pow` and `**` as V8 15.2 computes them: the ECMAScript special cases
//! of `v8::internal::math::pow` (`src/numbers/ieee754.cc`), then, because V8's
//! `--use-std-math-pow` defaults to true on every platform but AIX, the C
//! library's `pow` of the platform it runs on.
//!
//! Native builds therefore call the platform `pow` too (`f64::powf`), which on
//! macOS is the same libsystem_m `pow` Electron's V8 calls: bit-identical to
//! the shipping editor. wasm32 has no platform `pow`; it uses the correctly
//! rounded `pxfm::f_pow`, which agrees with macOS's `pow` on 99.91% of
//! measured inputs (Apple's `pow` is off by one unit in the last place on the
//! rest). See `docs/engine/expressions.md`.

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

#[cfg(target_family = "wasm")]
fn platform_pow(x: f64, y: f64) -> f64 {
    pxfm::f_pow(x, y)
}
