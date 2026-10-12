//! `atan` ported line for line from LLVM libc
//! (`libc/src/__support/math/atan.h` and `atan_utils.h` at
//! 3ea89f4304312567e31a8eb45e0737577e65b676), built with the accurate pass and
//! FMA, as V8 15.2 in Electron 44 runs it on arm64. It is not correctly
//! rounded (for |x| < 2^-26 it returns `fma(-2^-54, x, x)`), so a correctly
//! rounded `atan` would differ from `Math.atan`.
//!
//! Part of the LLVM Project, under the Apache License v2.0 with LLVM Exceptions.
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use super::dd::{DoubleDouble, div, exact_add, exact_mult, fma, nearest_integer};
use super::hexf::hexf;
use super::tables::ATAN_I;

/// `atan_eval`: degree-9 Taylor polynomial in double-double for |x| <= 2^-7.
fn atan_eval(x: DoubleDouble) -> DoubleDouble {
    let x_hi_sq = x.hi * x.hi;
    let c0 = fma(
        x_hi_sq,
        hexf("0x1.999999999999ap-3"),
        hexf("-0x1.5555555555555p-2"),
    );
    let c1 = fma(
        x_hi_sq,
        hexf("0x1.c71c71c71c71cp-4"),
        hexf("-0x1.2492492492492p-3"),
    );
    let x_hi_3 = x_hi_sq * x.hi;
    let x_hi_4 = x_hi_sq * x_hi_sq;
    let d0 = fma(x_hi_4, c1, c0);
    let d1 = fma(x_hi_4 - x_hi_sq, x.lo, x.lo);
    DoubleDouble {
        hi: x.hi,
        lo: fma(x_hi_3, d0, d1),
    }
}

const IS_NEG: [f64; 2] = [1.0, -1.0];
const PI_OVER_2: DoubleDouble = DoubleDouble {
    lo: hexf("0x1.1a62633145c07p-54"),
    hi: hexf("0x1.921fb54442d18p0"),
};
const MPI_OVER_2: DoubleDouble = DoubleDouble {
    lo: hexf("-0x1.1a62633145c07p-54"),
    hi: hexf("-0x1.921fb54442d18p0"),
};

pub(crate) fn atan(x: f64) -> f64 {
    let bits = x.to_bits();
    let x_sign = (bits >> 63) as usize;
    let x_abs = bits & 0x7fff_ffff_ffff_ffff;
    let x_exp = (x_abs >> 52) as i32 - 1023;

    // |x| < 1.
    if x_exp < 0 {
        if x_exp < -26 {
            if x == 0.0 {
                return x;
            }
            // |x| < 2^-26
            return fma(hexf("-0x1.0p-54"), x, x);
        }

        let x_d = f64::from_bits(x_abs);
        // k = 2^-6 * round(2^6 * |x|)
        let mut k = nearest_integer(hexf("0x1.0p6") * x_d);
        let idx = k as usize;
        k *= hexf("0x1.0p-6");

        // numerator = |x| - k
        let num = DoubleDouble {
            lo: 0.0,
            hi: x_d - k,
        };
        // denominator = 1 - k * |x|
        let den_hi = fma(x_d, k, 1.0);
        let prod = exact_mult(x_d, k);
        let den = DoubleDouble {
            hi: den_hi,
            lo: ((1.0 - den_hi) + prod.hi) + prod.lo,
        };

        let x_r = div(num, den);
        let p = atan_eval(x_r);

        let c0 = exact_add(ATAN_I[idx].hi, p.hi);
        let c1 = c0.lo + (ATAN_I[idx].lo + p.lo);
        return IS_NEG[x_sign] * (c0.hi + c1);
    }

    // |x| >= 2^53 or x is NaN.
    if x_exp >= 53 {
        if x.is_nan() {
            // V8 sees quiet NaNs only; a signaling NaN would be quieted here.
            return f64::NAN;
        }
        return fma(IS_NEG[x_sign], PI_OVER_2.hi, IS_NEG[x_sign] * PI_OVER_2.lo);
    }

    let x_d = f64::from_bits(x_abs);
    let y = 1.0 / x_d;

    // k = 2^-6 * round(2^6 / |x|)
    let mut k = nearest_integer(hexf("0x1.0p6") * y);
    let idx = k as usize;
    k *= hexf("0x1.0p-6");

    // denominator = |x| + k
    let den = exact_add(x_d, k);
    // numerator = 1 - k * |x|
    let num_hi = fma(-x_d, k, 1.0);
    let prod = exact_mult(x_d, k);
    let num = DoubleDouble {
        hi: num_hi,
        lo: ((1.0 - num_hi) - prod.hi) - prod.lo,
    };

    let x_r = div(num, den);
    let p = atan_eval(x_r);

    let c0 = exact_add(MPI_OVER_2.hi, ATAN_I[idx].hi);
    let c1 = exact_add(c0.hi, p.hi);
    let c2 = c1.lo + (c0.lo + p.lo) + (ATAN_I[idx].lo + MPI_OVER_2.lo);

    IS_NEG[1 - x_sign] * (c1.hi + c2)
}
