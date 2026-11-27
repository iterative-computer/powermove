//! `Math.exp`: CORE-MATH's correctly rounded binary64 `exp` (Paul Zimmermann
//! and Tom Hertz, MIT licence), as ported to Rust by pxfm 0.1.30
//! (`src/exponents/exp.rs`, Radzivon Bartoshyk, BSD-3-Clause OR Apache-2.0),
//! with two changes:
//!
//! * pxfm's accurate path for subnormal results drops the high part of the
//!   final two-sum (`f.hi` is not updated), returning wrong subnormals such as
//!   exp(-709.6296676644124) = 3.68e-309 instead of 6.48e-309. CORE-MATH and
//!   this port assign it.
//! * Every multiply-add is fused (`f64::mul_add`) on every target, so wasm32
//!   computes exactly what arm64 does.
//!
//! V8 15.2 computes `Math.exp` with LLVM libc's correctly rounded `exp`, so the
//! two agree on every input.
//!
//! The table pairs below are `(lo, hi)` bit patterns copied from pxfm.

use super::dd::{DoubleDouble, exact_add, exact_mult, fma};

static EXP_REDUCE_T0: [(u64, u64); 64] = [
    (0x0000000000000000, 0x3ff0000000000000),
    (0xbc719083535b085e, 0x3ff02c9a3e778061),
    (0x3c8d73e2a475b466, 0x3ff059b0d3158574),
    (0x3c6186be4bb28500, 0x3ff0874518759bc8),
    (0x3c98a62e4adc610a, 0x3ff0b5586cf9890f),
    (0x3c403a1727c57b52, 0x3ff0e3ec32d3d1a2),
    (0xbc96c51039449b3a, 0x3ff11301d0125b51),
    (0xbc932fbf9af1369e, 0x3ff1429aaea92de0),
    (0xbc819041b9d78a76, 0x3ff172b83c7d517b),
    (0x3c8e5b4c7b4968e4, 0x3ff1a35beb6fcb75),
    (0x3c9e016e00a2643c, 0x3ff1d4873168b9aa),
    (0x3c8dc775814a8494, 0x3ff2063b88628cd6),
    (0x3c99b07eb6c70572, 0x3ff2387a6e756238),
    (0x3c82bd339940e9da, 0x3ff26b4565e27cdd),
    (0x3c8612e8afad1256, 0x3ff29e9df51fdee1),
    (0x3c90024754db41d4, 0x3ff2d285a6e4030b),
    (0x3c86f46ad23182e4, 0x3ff306fe0a31b715),
    (0x3c932721843659a6, 0x3ff33c08b26416ff),
    (0xbc963aeabf42eae2, 0x3ff371a7373aa9cb),
    (0xbc75e436d661f5e2, 0x3ff3a7db34e59ff7),
    (0x3c8ada0911f09ebc, 0x3ff3dea64c123422),
    (0xbc5ef3691c309278, 0x3ff4160a21f72e2a),
    (0x3c489b7a04ef80d0, 0x3ff44e086061892d),
    (0x3c73c1a3b69062f0, 0x3ff486a2b5c13cd0),
    (0x3c7d4397afec42e2, 0x3ff4bfdad5362a27),
    (0xbc94b309d25957e4, 0x3ff4f9b2769d2ca7),
    (0xbc807abe1db13cac, 0x3ff5342b569d4f82),
    (0x3c99bb2c011d93ac, 0x3ff56f4736b527da),
    (0x3c96324c054647ac, 0x3ff5ab07dd485429),
    (0x3c9ba6f93080e65e, 0x3ff5e76f15ad2148),
    (0xbc9383c17e40b496, 0x3ff6247eb03a5585),
    (0xbc9bb60987591c34, 0x3ff6623882552225),
    (0xbc9bdd3413b26456, 0x3ff6a09e667f3bcd),
    (0xbc6bbe3a683c88aa, 0x3ff6dfb23c651a2f),
    (0xbc816e4786887a9a, 0x3ff71f75e8ec5f74),
    (0xbc90245957316dd4, 0x3ff75feb564267c9),
    (0xbc841577ee049930, 0x3ff7a11473eb0187),
    (0x3c705d02ba15797e, 0x3ff7e2f336cf4e62),
    (0xbc9d4c1dd41532d8, 0x3ff82589994cce13),
    (0xbc9fc6f89bd4f6ba, 0x3ff868d99b4492ed),
    (0x3c96e9f156864b26, 0x3ff8ace5422aa0db),
    (0x3c85cc13a2e3976c, 0x3ff8f1ae99157736),
    (0xbc675fc781b57ebc, 0x3ff93737b0cdc5e5),
    (0xbc9d185b7c1b85d0, 0x3ff97d829fde4e50),
    (0x3c7c7c46b071f2be, 0x3ff9c49182a3f090),
    (0xbc9359495d1cd532, 0x3ffa0c667b5de565),
    (0xbc9d2f6edb8d41e2, 0x3ffa5503b23e255d),
    (0x3c90fac90ef7fd32, 0x3ffa9e6b5579fdbf),
    (0x3c97a1cd345dcc82, 0x3ffae89f995ad3ad),
    (0xbc62805e3084d708, 0x3ffb33a2b84f15fb),
    (0xbc75584f7e54ac3a, 0x3ffb7f76f2fb5e47),
    (0x3c823dd07a2d9e84, 0x3ffbcc1e904bc1d2),
    (0x3c811065895048de, 0x3ffc199bdd85529c),
    (0x3c92884dff483cac, 0x3ffc67f12e57d14b),
    (0x3c7503cbd1e949dc, 0x3ffcb720dcef9069),
    (0xbc9cbc3743797a9c, 0x3ffd072d4a07897c),
    (0x3c82ed02d75b3706, 0x3ffd5818dcfba487),
    (0x3c9c2300696db532, 0x3ffda9e603db3285),
    (0xbc91a5cd4f184b5c, 0x3ffdfc97337b9b5f),
    (0x3c839e8980a9cc90, 0x3ffe502ee78b3ff6),
    (0xbc9e9c23179c2894, 0x3ffea4afa2a490da),
    (0x3c9dc7f486a4b6b0, 0x3ffefa1bee615a27),
    (0x3c99d3e12dd8a18a, 0x3fff50765b6e4540),
    (0x3c874853f3a5931e, 0x3fffa7c1819e90d8),
];

static EXP_REDUCE_T1: [(u64, u64); 64] = [
    (0x0000000000000000, 0x3ff0000000000000),
    (0x3c9ae8e38c59c72a, 0x3ff000b175effdc7),
    (0xbc57b5d0d58ea8f4, 0x3ff00162f3904052),
    (0x3c94115cb6b16a8e, 0x3ff0021478e11ce6),
    (0xbc8d7c96f201bb2e, 0x3ff002c605e2e8cf),
    (0x3c984711d4c35ea0, 0x3ff003779a95f959),
    (0xbc80484245243778, 0x3ff0042936faa3d8),
    (0xbc94b237da2025fa, 0x3ff004dadb113da0),
    (0xbc75e00e62d6b30e, 0x3ff0058c86da1c0a),
    (0x3c9a1d6cedbb9480, 0x3ff0063e3a559473),
    (0xbc94acf197a00142, 0x3ff006eff583fc3d),
    (0xbc6eaf2ea42391a6, 0x3ff007a1b865a8ca),
    (0x3c7da93f90835f76, 0x3ff0085382faef83),
    (0xbc86a79084ab093c, 0x3ff00905554425d4),
    (0x3c986364f8fbe8f8, 0x3ff009b72f41a12b),
    (0xbc882e8e14e3110e, 0x3ff00a6910f3b6fd),
    (0xbc84f6b2a7609f72, 0x3ff00b1afa5abcbf),
    (0xbc7e1a258ea8f71a, 0x3ff00bcceb7707ec),
    (0x3c74362ca5bc26f2, 0x3ff00c7ee448ee02),
    (0x3c9095a56c919d02, 0x3ff00d30e4d0c483),
    (0xbc6406ac4e81a646, 0x3ff00de2ed0ee0f5),
    (0x3c9b5a6902767e08, 0x3ff00e94fd0398e0),
    (0xbc991b2060859320, 0x3ff00f4714af41d3),
    (0x3c8427068ab22306, 0x3ff00ff93412315c),
    (0x3c9c1d0660524e08, 0x3ff010ab5b2cbd11),
    (0xbc9e7bdfb3204be8, 0x3ff0115d89ff3a8b),
    (0x3c8843aa8b9cbbc6, 0x3ff0120fc089ff63),
    (0xbc734104ee7edae8, 0x3ff012c1fecd613b),
    (0xbc72b6aeb6176892, 0x3ff0137444c9b5b5),
    (0x3c7a8cd33b8a1bb2, 0x3ff01426927f5278),
    (0x3c72edc08e5da99a, 0x3ff014d8e7ee8d2f),
    (0x3c857ba2dc7e0c72, 0x3ff0158b4517bb88),
    (0x3c9b61299ab8cdb8, 0x3ff0163da9fb3335),
    (0xbc990565902c5f44, 0x3ff016f0169949ed),
    (0x3c870fc41c5c2d54, 0x3ff017a28af25567),
    (0x3c94b9a6e145d76c, 0x3ff018550706ab62),
    (0xbc7008eff5142bfa, 0x3ff019078ad6a19f),
    (0xbc977669f033c7de, 0x3ff019ba16628de2),
    (0xbc909bb78eeead0a, 0x3ff01a6ca9aac5f3),
    (0x3c9371231477ece6, 0x3ff01b1f44af9f9e),
    (0x3c75e7626621eb5a, 0x3ff01bd1e77170b4),
    (0xbc9bc72b100828a4, 0x3ff01c8491f08f08),
    (0xbc6ce39cbbab8bbe, 0x3ff01d37442d5070),
    (0x3c816996709da2e2, 0x3ff01de9fe280ac8),
    (0xbc8c11f5239bf536, 0x3ff01e9cbfe113ef),
    (0x3c8e1d4eb5edc6b4, 0x3ff01f4f8958c1c6),
    (0xbc9afb99946ee3f0, 0x3ff020025a8f6a35),
    (0xbc98f06d8a148a32, 0x3ff020b533856324),
    (0xbc82bf310fc54eb6, 0x3ff02168143b0281),
    (0xbc9c95a035eb4176, 0x3ff0221afcb09e3e),
    (0xbc9491793e46834c, 0x3ff022cdece68c4f),
    (0xbc73e8d0d9c49090, 0x3ff02380e4dd22ad),
    (0xbc9314aa16278aa4, 0x3ff02433e494b755),
    (0x3c848daf888e9650, 0x3ff024e6ec0da046),
    (0x3c856dc8046821f4, 0x3ff02599fb483385),
    (0x3c945b42356b9d46, 0x3ff0264d1244c719),
    (0xbc7082ef51b61d7e, 0x3ff027003103b10e),
    (0x3c72106ed0920a34, 0x3ff027b357854772),
    (0xbc9fd4cf26ea5d0e, 0x3ff0286685c9e059),
    (0xbc909f8775e78084, 0x3ff02919bbd1d1d8),
    (0x3c564cbba902ca28, 0x3ff029ccf99d720a),
    (0x3c94383ef231d206, 0x3ff02a803f2d170d),
    (0x3c94a47a505b3a46, 0x3ff02b338c811703),
    (0x3c9e471202234680, 0x3ff02be6e199c811),
];

fn pair(bits: (u64, u64)) -> DoubleDouble {
    DoubleDouble {
        lo: f64::from_bits(bits.0),
        hi: f64::from_bits(bits.1),
    }
}

fn quick_mult(a: DoubleDouble, b: DoubleDouble) -> DoubleDouble {
    let r = exact_mult(a.hi, b.hi);
    let t1 = fma(a.hi, b.lo, r.lo);
    DoubleDouble {
        hi: r.hi,
        lo: fma(a.lo, b.hi, t1),
    }
}

fn quick_mult_f64(a: DoubleDouble, b: f64) -> DoubleDouble {
    let hi = b * a.hi;
    DoubleDouble {
        hi,
        lo: fma(b, a.lo, fma(b, a.hi, -hi)),
    }
}

/// 2Sum (no ordering requirement).
fn full_exact_add(a: f64, b: f64) -> DoubleDouble {
    let hi = a + b;
    let t1 = hi - a;
    let t2 = hi - t1;
    let t3 = b - t1;
    let t4 = a - t2;
    DoubleDouble { hi, lo: t3 + t4 }
}

fn full_add_f64(a: DoubleDouble, b: f64) -> DoubleDouble {
    let t = full_exact_add(a.hi, b);
    DoubleDouble {
        hi: t.hi,
        lo: a.lo + t.lo,
    }
}

fn add(a: DoubleDouble, b: DoubleDouble) -> DoubleDouble {
    let s = a.hi + b.hi;
    let d = s - a.hi;
    let l = ((b.hi - d) + (a.hi + (d - s))) + (a.lo + b.lo);
    DoubleDouble { hi: s, lo: l }
}

fn add_f64(a: DoubleDouble, b: f64) -> DoubleDouble {
    let t = exact_add(a.hi, b);
    DoubleDouble {
        hi: t.hi,
        lo: a.lo + t.lo,
    }
}

fn quick_mul_add(a: DoubleDouble, b: DoubleDouble, c: DoubleDouble) -> DoubleDouble {
    let DoubleDouble { hi: h, lo: r } = quick_mult(a, b);
    let DoubleDouble { hi: p, lo: q } = add_f64(c, h);
    DoubleDouble { hi: p, lo: r + q }
}

fn quick_mul_add_f64(a: DoubleDouble, b: DoubleDouble, c: f64) -> DoubleDouble {
    let DoubleDouble { hi: h, lo: r } = quick_mult(a, b);
    let DoubleDouble { hi: p, lo: q } = exact_add(c, h);
    DoubleDouble { hi: p, lo: r + q }
}

/// Keeps the fraction bits: the subnormal whose significand they are.
fn to_denormal(x: f64) -> f64 {
    f64::from_bits(x.to_bits() & 0x000f_ffff_ffff_ffff)
}

/// Adds `i` to the exponent field (no overflow checks; callers stay in range).
fn fast_ldexp(d: f64, i: i32) -> f64 {
    f64::from_bits(d.to_bits().wrapping_add((i as u64).wrapping_shl(52)))
}

fn exp_poly_dd(z: DoubleDouble) -> DoubleDouble {
    const C: [(u64, u64); 7] = [
        (0x0000000000000000, 0x3ff0000000000000),
        (0x39c712f72ecec2cf, 0x3fe0000000000000),
        (0x3c65555555554d07, 0x3fc5555555555555),
        (0x3c455194d28275da, 0x3fa5555555555555),
        (0x3c012faa0e1c0f7b, 0x3f81111111111111),
        (0xbbf4ba45ab25d2a3, 0x3f56c16c16da6973),
        (0xbbc9091d845ecd36, 0x3f2a01a019eb7f31),
    ];
    let mut r = quick_mul_add(pair(C[6]), z, pair(C[5]));
    r = quick_mul_add(r, z, pair(C[4]));
    r = quick_mul_add(r, z, pair(C[3]));
    r = quick_mul_add(r, z, pair(C[2]));
    r = quick_mul_add(r, z, pair(C[1]));
    quick_mul_add_f64(r, z, 1.0)
}

const L2: DoubleDouble = DoubleDouble {
    lo: f64::from_bits(0x3d0718432a1b0e26),
    hi: f64::from_bits(0x3f262e42ff000000),
};

#[cold]
fn exp_accurate(x: f64, t: f64, tz: DoubleDouble, ie: i64) -> f64 {
    let mut ix = x.to_bits();
    if ((ix >> 52) & 0x7ff) < 0x3c9 {
        return 1.0 + x;
    }
    const L2LL: f64 = f64::from_bits(0x3999ff0342542fc3);
    let dx = fma(-L2.hi, t, x);
    let dx_dd = quick_mult_f64(
        DoubleDouble {
            lo: L2LL,
            hi: L2.lo,
        },
        t,
    );
    let dz = full_add_f64(dx_dd, dx);
    let mut f = exp_poly_dd(dz);
    f = quick_mult(dz, f);
    if ix > 0xc086232bdd7abcd2 {
        // x < -708.396: subnormal result.
        ix = 1i64.wrapping_sub(ie).wrapping_shl(52) as u64;
        f = quick_mult(f, tz);
        f = add(tz, f);
        let sum = exact_add(f64::from_bits(ix), f.hi);
        f.hi = sum.hi;
        f.lo += sum.lo;
        f.hi = to_denormal(f.hi + f.lo);
    } else {
        if tz.hi == 1.0 {
            let fhe = exact_add(tz.hi, f.hi);
            let fhl = exact_add(fhe.lo, f.lo);
            f.hi = fhe.hi;
            f.lo = fhl.hi;
            ix = f.lo.to_bits();
            if (ix & 0x000f_ffff_ffff_ffff) == 0 {
                let v = fhl.lo.to_bits();
                let d: i64 = (((((ix as i64) >> 63) ^ ((v as i64) >> 63)) as u64).wrapping_shl(1)
                    as i64)
                    .wrapping_add(1);
                ix = ix.wrapping_add(d as u64);
                f.lo = f64::from_bits(ix);
            }
        } else {
            f = quick_mult(f, tz);
            f = add(tz, f);
        }
        f = exact_add(f.hi, f.lo);
        f.hi = fast_ldexp(f.hi, ie as i32);
    }
    f.hi
}

pub(crate) fn exp(x: f64) -> f64 {
    let mut ix = x.to_bits();
    let aix = ix & 0x7fff_ffff_ffff_ffff;
    // exp(x) rounds to 1 for |x| <= 5.55112e-17
    if aix <= 0x3c90000000000000 {
        return 1.0 + x;
    }
    if aix >= 0x40862e42fefa39f0 {
        // |x| >= 709.783
        if aix > 0x7ff0000000000000 {
            return x + x; // NaN
        }
        if aix == 0x7ff0000000000000 {
            return if (ix >> 63) != 0 { 0.0 } else { x };
        }
        if (ix >> 63) == 0 {
            let z = f64::from_bits(0x7fe0000000000000);
            return z * z; // overflow to +inf
        }
        if aix >= 0x40874910d52d3052 {
            // x <= -745.133: underflow to +0
            return f64::from_bits(0x18000000000000) * f64::from_bits(0x3c80000000000000);
        }
    }
    const S: f64 = f64::from_bits(0x40b71547652b82fe);
    let t = (x * S).round_ties_even();
    let jt = t as i64;
    let i0 = ((jt >> 6) & 0x3f) as usize;
    let i1 = (jt & 0x3f) as usize;
    let ie = jt >> 12;
    let tz = quick_mult(pair(EXP_REDUCE_T0[i0]), pair(EXP_REDUCE_T1[i1]));

    // Cody-Waite reduction: |t| < 2^23 and L2.hi has 29 bits, so L2.hi*t is exact.
    let dx = fma(L2.lo, t, fma(-L2.hi, t, x));
    let dx2 = dx * dx;
    const CH: [u64; 4] = [
        0x3ff0000000000000,
        0x3fe0000000000000,
        0x3fc55555557e54ff,
        0x3fa55555553a12f4,
    ];
    let pw0 = fma(dx, f64::from_bits(CH[3]), f64::from_bits(CH[2]));
    let pw1 = fma(dx, f64::from_bits(CH[1]), f64::from_bits(CH[0]));
    let p = fma(dx2, pw0, pw1);
    let mut f = DoubleDouble {
        hi: tz.hi,
        lo: fma(tz.hi * dx, p, tz.lo),
    };
    const EPS: f64 = f64::from_bits(0x3c0833beace2b6fe);
    if ix > 0xc086232bdd7abcd2 {
        // subnormal case: x < -708.396
        ix = 1u64.wrapping_sub(ie as u64).wrapping_shl(52);
        let sum = exact_add(f64::from_bits(ix), f.hi);
        f.hi = sum.hi;
        f.lo += sum.lo;
        let ub = f.hi + (f.lo + EPS);
        let lb = f.hi + (f.lo - EPS);
        if ub != lb {
            return exp_accurate(x, t, tz, ie);
        }
        to_denormal(lb)
    } else {
        let ub = f.hi + (f.lo + EPS);
        let lb = f.hi + (f.lo - EPS);
        if ub != lb {
            return exp_accurate(x, t, tz, ie);
        }
        fast_ldexp(lb, ie as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::exp;

    #[test]
    fn subnormal_accurate_path() {
        // V8 15.2 (LLVM libc, correctly rounded); pxfm 0.1.30 returns 0x0002a56312b5ef4c.
        assert_eq!(exp(-709.6296676644124).to_bits(), 0x0004a958c4ad7bd3);
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(1.0), std::f64::consts::E);
        assert_eq!(exp(f64::NEG_INFINITY), 0.0);
        assert_eq!(exp(f64::INFINITY), f64::INFINITY);
        assert_eq!(exp(710.0), f64::INFINITY);
        assert_eq!(exp(-746.0), 0.0);
        assert!(exp(f64::NAN).is_nan());
    }
}
