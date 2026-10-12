//! The subset of LLVM libc's `FPUtil/double_double.h` that `atan` and `pow`
//! use, in the configuration Chromium builds for arm64 (`LIBC_TARGET_CPU_HAS_FMA`):
//! `multiply_add` is a fused multiply-add. `f64::mul_add` is fused on every
//! Rust target (software on wasm32), so the results are the same everywhere.
//!
//! Part of the LLVM Project, under the Apache License v2.0 with LLVM Exceptions.
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

/// `NumberPair<double>`: an unevaluated sum `hi + lo`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DoubleDouble {
    pub lo: f64,
    pub hi: f64,
}

/// `fputil::multiply_add` with a hardware FMA.
#[inline]
pub(crate) fn fma(x: f64, y: f64, z: f64) -> f64 {
    x.mul_add(y, z)
}

/// `fputil::exact_add` (Fast2Sum; requires |a| >= |b| or a == 0).
#[inline]
pub(crate) fn exact_add(a: f64, b: f64) -> DoubleDouble {
    let hi = a + b;
    let t = hi - a;
    DoubleDouble { hi, lo: b - t }
}

/// `fputil::exact_mult` on an FMA target.
#[inline]
pub(crate) fn exact_mult(a: f64, b: f64) -> DoubleDouble {
    let hi = a * b;
    DoubleDouble { hi, lo: fma(a, b, -hi) }
}

/// `fputil::div` on an FMA target.
#[inline]
pub(crate) fn div(a: DoubleDouble, b: DoubleDouble) -> DoubleDouble {
    let q = 1.0 / b.hi;
    let hi = a.hi * q;
    let e_hi = fma(b.hi, -hi, a.hi);
    let e_lo = fma(b.lo, -hi, a.lo);
    DoubleDouble { hi, lo: q * (e_hi + e_lo) }
}

/// `fputil::polyeval(x, a0, a1, a2)`: Horner's scheme with fused steps.
#[inline]
pub(crate) fn polyeval3(x: f64, a0: f64, a1: f64, a2: f64) -> f64 {
    fma(x, fma(x, a2, a1), a0)
}

/// `fputil::nearest_integer`: `rint` in the default rounding mode.
#[inline]
pub(crate) fn nearest_integer(x: f64) -> f64 {
    x.round_ties_even()
}
