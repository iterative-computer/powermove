//! `Math.pow` and `**` as V8 15.2 computes them: the ECMAScript special cases
//! of `v8::internal::math::pow` (`src/numbers/ieee754.cc`) in front of LLVM
//! libc's `pow` (`libc/src/__support/math/pow.h` at
//! 3ea89f4304312567e31a8eb45e0737577e65b676), ported line for line in its FMA
//! configuration. LLVM's `pow` is not correctly rounded (errors up to about
//! 0.505 ulp were measured), so only this exact algorithm reproduces V8.
//!
//! Part of the LLVM Project, under the Apache License v2.0 with LLVM Exceptions.
//! SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception

use super::dd::{DoubleDouble, exact_add, exact_mult, fma, nearest_integer, polyeval3};
use super::hexf::hexf;
use super::tables::{EXP2_MID1, LOG2_R_DD, RD};

const EXP_MASK: u64 = 0x7ff0_0000_0000_0000;
const ONE: u64 = 0x3ff0_0000_0000_0000;
const INF: u64 = 0x7ff0_0000_0000_0000;
const NEG_INF: u64 = 0xfff0_0000_0000_0000;
const MIN_NORMAL: u64 = 0x0010_0000_0000_0000;
const UNIT_EXPONENT: u32 = 1023 + 52;

fn biased_exponent(bits: u64) -> u32 {
    ((bits >> 52) & 0x7ff) as u32
}

fn is_odd_integer(x: f64) -> bool {
    let x_u = x.to_bits();
    let x_e = biased_exponent(x_u);
    let lsb = (x_u | EXP_MASK).trailing_zeros();
    x_e + lsb == UNIT_EXPONENT
}

fn is_integer(x: f64) -> bool {
    let x_u = x.to_bits();
    let x_e = biased_exponent(x_u);
    let lsb = (x_u | EXP_MASK).trailing_zeros();
    x_e + lsb >= UNIT_EXPONENT
}

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
    llvm_pow(x, y)
}

/// LLVM libc `pow` (FMA build, accurate pass). Signaling NaNs cannot reach it
/// from JavaScript, so the FE_INVALID path is not modelled.
fn llvm_pow(x: f64, mut y: f64) -> f64 {
    let xbits = x.to_bits();
    let ybits = y.to_bits();
    let y_sign = ybits >> 63 != 0;
    let x_sign = xbits >> 63 != 0;
    let mut x_mant = xbits & 0x000f_ffff_ffff_ffff;
    let y_mant = ybits & 0x000f_ffff_ffff_ffff;
    let x_u = xbits;
    let x_a = xbits & 0x7fff_ffff_ffff_ffff;
    let y_a = ybits & 0x7fff_ffff_ffff_ffff;

    let mut e_x = biased_exponent(xbits) as f64 - 1023.0;
    let mut sign: u64 = 0;

    if y_mant == 0 || y_a > 0x43d7_4910_d52d_3052 || x_u == ONE || x_u >= INF || x_u < MIN_NORMAL {
        if y == 0.0 {
            return 1.0;
        }
        match y_a {
            0x3fe0_0000_0000_0000 => {
                // y = +-0.5
                if x == 0.0 || x_u == NEG_INF {
                    return if y_sign { 1.0 / (x * x) } else { x * x };
                }
                return if y_sign { 1.0 / x.sqrt() } else { x.sqrt() };
            }
            0x3ff0_0000_0000_0000 => return if y_sign { 1.0 / x } else { x },
            0x4000_0000_0000_0000 => return if y_sign { 1.0 / (x * x) } else { x * x },
            _ => {}
        }

        if y_a > 0x43d7_4910_d52d_3052 {
            if y_a >= INF {
                if y_mant != 0 {
                    // y is NaN
                    return if x_u == ONE { 1.0 } else { y };
                }
                // y is +-Inf
                if x.is_nan() {
                    return x;
                }
                if x_a == ONE {
                    return 1.0;
                }
                if x == 0.0 && y_sign {
                    return f64::INFINITY;
                }
                return if (x_a < ONE) == y_sign { f64::INFINITY } else { 0.0 };
            }
            y = if y_sign { -hexf("0x1.0p100") } else { hexf("0x1.0p100") };
        }

        if x_u == ONE {
            return 1.0;
        }

        if x == 0.0 {
            let out_is_neg = x_sign && is_odd_integer(y);
            if y_sign {
                return if out_is_neg { f64::NEG_INFINITY } else { f64::INFINITY };
            }
            return if out_is_neg { -0.0 } else { 0.0 };
        }

        if x_a == INF {
            let out_is_neg = x_sign && is_odd_integer(y);
            if y_sign {
                return if out_is_neg { -0.0 } else { 0.0 };
            }
            return if out_is_neg { f64::NEG_INFINITY } else { f64::INFINITY };
        }

        if x_a > INF {
            return x;
        }

        if x_a < MIN_NORMAL {
            let x_norm = (x * hexf("0x1.0p64")).to_bits();
            e_x = (biased_exponent(x_norm) as f64 - 1023.0) - 64.0;
            x_mant = x_norm & 0x000f_ffff_ffff_ffff;
        }

        if x_sign {
            // LLVM negates x here; only its (sign-free) bit fields are used below.
            if is_integer(y) {
                if is_odd_integer(y) {
                    sign = 0x8000_0000_0000_0000;
                }
            } else {
                return f64::NAN;
            }
        }
    }

    let idx_x = (x_mant >> (52 - 7)) as usize;
    let m_x = f64::from_bits(x_mant | ONE);

    const COEFFS: [f64; 7] = [
        hexf("0x1.71547652b82fep0"),
        hexf("-0x1.71547652b82e7p-1"),
        hexf("0x1.ec709dc3b1fd5p-2"),
        hexf("-0x1.7154766124215p-2"),
        hexf("0x1.2776bd90259d8p-2"),
        hexf("-0x1.ec586c6f3d311p-3"),
        hexf("0x1.9c4775eccf524p-3"),
    ];

    let dx = fma(RD[idx_x], m_x, -1.0); // Exact
    let dx_c0 = exact_mult(COEFFS[0], dx);

    let dx2 = dx * dx;
    let c0 = fma(dx, COEFFS[2], COEFFS[1]);
    let c1 = fma(dx, COEFFS[4], COEFFS[3]);
    let c2 = fma(dx, COEFFS[6], COEFFS[5]);

    let p = polyeval3(dx2, c0, c1, c2);

    let log2_x_hi = exact_add(e_x + LOG2_R_DD[idx_x].hi, dx_c0.hi);
    let log2_x_lo = fma(dx2, p, dx_c0.lo + LOG2_R_DD[idx_x].lo);
    let mut log2_x: DoubleDouble = exact_add(log2_x_hi.hi, log2_x_lo);
    log2_x.lo += log2_x_hi.lo;

    let y6 = y * hexf("0x1.0p6"); // Exact.

    let mut y6_log2_x = exact_mult(y6, log2_x.hi);
    y6_log2_x.lo = fma(y6, log2_x.lo, y6_log2_x.lo);

    let mut scale = 1.0;
    const UPPER_EXP_BOUND: f64 = 512.0 * 64.0;
    if y6_log2_x.hi.abs() >= UPPER_EXP_BOUND {
        if y6_log2_x.hi.to_bits() >> 63 == 0 {
            scale = hexf("0x1.0p512");
            y6_log2_x.hi -= 512.0 * 64.0;
            if y6_log2_x.hi > 513.0 * 64.0 {
                y6_log2_x.hi = 513.0 * 64.0;
            }
        } else {
            scale = hexf("0x1.0p-512");
            y6_log2_x.hi += 512.0 * 64.0;
            if y6_log2_x.hi < (-1076.0 + 512.0) * 64.0 {
                y6_log2_x.hi = -564.0 * 64.0;
            }
        }
    }

    let hm = nearest_integer(y6_log2_x.hi);

    let lo6_hi = y6_log2_x.hi - hm;
    let lo6 = lo6_hi + y6_log2_x.lo;

    let hm_i = hm as i32;
    let idx_y = (hm_i as u32 & 0x3f) as usize;

    let exp2_hi_i = (((hm_i >> 6) as i64 as u64) << 52) as i64;
    let exp2_mid_hi_i = EXP2_MID1[idx_y].1.to_bits() as i64;
    let exp2_mid_lo_i = EXP2_MID1[idx_y].0.to_bits() as i64;
    let exp2_hm_hi_i = (exp2_hi_i.wrapping_add(exp2_mid_hi_i) as u64).wrapping_add(sign);
    let exp2_hm_lo_i = if idx_y != 0 {
        (exp2_hi_i.wrapping_add(exp2_mid_lo_i) as u64).wrapping_add(sign)
    } else {
        sign
    };
    let exp2_hm_hi = f64::from_bits(exp2_hm_hi_i);
    let exp2_hm_lo = f64::from_bits(exp2_hm_lo_i);

    const EXP2_COEFFS: [f64; 6] = [
        hexf("0x1p0"),
        hexf("0x1.62e42fefa39efp-7"),
        hexf("0x1.ebfbdff82a23ap-15"),
        hexf("0x1.c6b08d7076268p-23"),
        hexf("0x1.3b2ad33f8b48bp-31"),
        hexf("0x1.5d870c4d84445p-40"),
    ];

    let lo6_sqr = lo6 * lo6;
    let d0 = fma(lo6, EXP2_COEFFS[2], EXP2_COEFFS[1]);
    let d1 = fma(lo6, EXP2_COEFFS[4], EXP2_COEFFS[3]);
    let pp = polyeval3(lo6_sqr, d0, d1, EXP2_COEFFS[5]);

    let mut r = fma(exp2_hm_hi * lo6, pp, exp2_hm_lo);
    r += exp2_hm_hi;

    r * scale
}
