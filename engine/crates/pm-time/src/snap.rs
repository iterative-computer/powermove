//! Document frame rates (section 5, "At the document boundary", rule 3).
//!
//! Rates come from `fpsRational` when present. Otherwise the stored float snaps:
//!
//! 1. within 0.02% of `k × 1000/1001` for `k` in {24, 30, 48, 60, 120, 240}: that
//!    NTSC rate;
//! 2. otherwise within 0.02% of an integer supported rate: that rate;
//! 3. otherwise the rational with the smallest denominator within 1e-9 of the
//!    float (absolute), computed exactly from the float's binary value.

use crate::convert::decompose;
use crate::{Rate, Rational, TICKS_PER_SECOND, TimeError};

/// The `k` in `k × 1000/1001` that snap to NTSC rates.
pub const NTSC_MULTIPLES: [i64; 6] = [24, 30, 48, 60, 120, 240];

/// Integer supported rates a float may snap to.
pub const SNAP_INTEGER_RATES: [i64; 10] = [24, 25, 30, 48, 50, 60, 90, 100, 120, 240];

/// 0.02%, relative to the candidate rate.
pub const SNAP_RELATIVE_TOLERANCE: f64 = 0.0002;

/// Absolute tolerance of the rational fallback.
const RATIONAL_TOLERANCE: f64 = 1e-9;

/// Where a document rate came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateSource {
    /// The document's `fpsRational`.
    Exact,
    /// The float snapped to `k × 1000/1001`.
    Ntsc,
    /// The float snapped to an integer supported rate.
    Integer,
    /// The smallest-denominator rational within 1e-9 of the float.
    Approximated,
}

/// A composition rate resolved from the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentRate {
    pub rate: Rate,
    pub source: RateSource,
}

/// Resolves a composition rate from the document's `fpsRational` (when present
/// and valid) or its float `fps`.
///
/// An `fpsRational` that is not a valid rate is ignored and the float decides;
/// [`RateSource`] records which rule applied.
pub fn rate_from_document(
    fps_rational: Option<[i64; 2]>,
    fps: f64,
) -> Result<DocumentRate, TimeError> {
    if let Some([num, den]) = fps_rational
        && let Ok(rate) = Rate::new(num, den)
    {
        return Ok(DocumentRate {
            rate,
            source: RateSource::Exact,
        });
    }
    snap_float_rate(fps)
}

/// Snaps a float frame rate by rules 1 to 3.
pub fn snap_float_rate(fps: f64) -> Result<DocumentRate, TimeError> {
    if !fps.is_finite() || fps <= 0.0 || fps > TICKS_PER_SECOND as f64 {
        return Err(TimeError::InvalidFloatRate);
    }
    for k in NTSC_MULTIPLES {
        let target = (k * 1000) as f64 / 1001.0;
        if (fps - target).abs() <= SNAP_RELATIVE_TOLERANCE * target {
            let rate = Rate::new(k * 1000, 1001)?;
            return Ok(DocumentRate {
                rate,
                source: RateSource::Ntsc,
            });
        }
    }
    for k in SNAP_INTEGER_RATES {
        let target = k as f64;
        if (fps - target).abs() <= SNAP_RELATIVE_TOLERANCE * target {
            return Ok(DocumentRate {
                rate: Rate::integer(k)?,
                source: RateSource::Integer,
            });
        }
    }
    let rational = simplest_rational_near(fps, RATIONAL_TOLERANCE)?;
    let rate = Rate::from_rational(rational).map_err(|_| TimeError::InvalidFloatRate)?;
    Ok(DocumentRate {
        rate,
        source: RateSource::Approximated,
    })
}

/// The rational with the smallest denominator (then smallest numerator) in the
/// closed interval `[value - tolerance, value + tolerance]`, evaluated exactly.
///
/// Requires `value > tolerance > 0`; fails when the exact endpoints exceed `i128`
/// or the result exceeds `i64`.
pub(crate) fn simplest_rational_near(value: f64, tolerance: f64) -> Result<Rational, TimeError> {
    if !(value.is_finite() && tolerance.is_finite() && tolerance > 0.0 && value > tolerance) {
        return Err(TimeError::InvalidFloatRate);
    }
    // Exact endpoints over a common power-of-two denominator.
    let (vm, ve) = decompose(value);
    let (tm, te) = decompose(tolerance);
    let shift = (-ve.min(te)).max(0) as u32;
    if shift > 120 {
        return Err(TimeError::InvalidFloatRate);
    }
    let scale = |m: i64, e: i32| -> Option<i128> {
        let e = e + shift as i32; // >= 0 by construction
        (m as i128).checked_mul(1i128.checked_shl(e as u32)?)
    };
    let v = scale(vm, ve).ok_or(TimeError::InvalidFloatRate)?;
    let t = scale(tm, te).ok_or(TimeError::InvalidFloatRate)?;
    let den = 1i128
        .checked_shl(shift)
        .ok_or(TimeError::InvalidFloatRate)?;
    let (lo, hi) = (v - t, v + t);
    let (p, q) = simplest_in_closed(lo, den, hi, den).ok_or(TimeError::Overflow)?;
    Rational::new(
        i64::try_from(p).map_err(|_| TimeError::Overflow)?,
        i64::try_from(q).map_err(|_| TimeError::Overflow)?,
    )
}

/// Simplest rational in `[a/b, c/d]` for `0 <= a/b <= c/d`, `b, d > 0`.
///
/// Continued-fraction descent (Stern–Brocot): the fraction with the smallest
/// denominator in the interval, which is unique.
fn simplest_in_closed(a: i128, b: i128, c: i128, d: i128) -> Option<(i128, i128)> {
    // floor of the lower bound
    let fl = a.div_euclid(b);
    if fl * b == a {
        return Some((fl, 1)); // the lower bound is an integer
    }
    // smallest integer > a/b is fl + 1; is it <= c/d ?
    if (fl + 1).checked_mul(d)? <= c {
        return Some((fl + 1, 1));
    }
    // Both bounds lie in (fl, fl + 1). Recurse on the reciprocals of the
    // fractional parts: x in [a/b, c/d] <=> 1/(x - fl) in [d/(c - fl d), b/(a - fl b)].
    let a2 = d;
    let b2 = c - fl * d;
    let c2 = b;
    let d2 = a - fl * b;
    if b2 == 0 {
        // c/d == fl exactly, impossible here since c/d > a/b > fl.
        return None;
    }
    let (p, q) = simplest_in_closed(a2, b2, c2, d2)?;
    Some((fl.checked_mul(p)?.checked_add(q)?, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(fps: f64) -> (Rate, RateSource) {
        let r = snap_float_rate(fps).unwrap();
        (r.rate, r.source)
    }

    #[test]
    fn ntsc_floats_snap_to_exact_ntsc() {
        assert_eq!(snap(29.97), (Rate::FPS_29_97, RateSource::Ntsc));
        assert_eq!(snap(30000.0 / 1001.0), (Rate::FPS_29_97, RateSource::Ntsc));
        assert_eq!(snap(29.97002997), (Rate::FPS_29_97, RateSource::Ntsc));
        assert_eq!(snap(23.976), (Rate::FPS_23_976, RateSource::Ntsc));
        assert_eq!(snap(23.98), (Rate::FPS_23_976, RateSource::Ntsc));
        assert_eq!(snap(47.952), (Rate::FPS_47_952, RateSource::Ntsc));
        assert_eq!(snap(59.94), (Rate::FPS_59_94, RateSource::Ntsc));
        assert_eq!(snap(119.88), (Rate::FPS_119_88, RateSource::Ntsc));
        assert_eq!(snap(239.76), (Rate::FPS_239_76, RateSource::Ntsc));
    }

    #[test]
    fn integer_floats_snap_to_integer_rates() {
        for k in SNAP_INTEGER_RATES {
            assert_eq!(
                snap(k as f64),
                (Rate::integer(k).unwrap(), RateSource::Integer)
            );
        }
        assert_eq!(snap(25.004), (Rate::FPS_25, RateSource::Integer));
        assert_eq!(snap(29.995), (Rate::FPS_30, RateSource::Integer));
    }

    #[test]
    fn tolerance_boundaries() {
        // 0.02% of 30 is 0.006 (compared in f64): 30.0059 snaps, 30.0061 does not.
        assert_eq!(snap(30.0059).1, RateSource::Integer);
        assert_eq!(snap(29.9941).1, RateSource::Integer);
        assert_eq!(snap(30.0061).1, RateSource::Approximated);
        // 29.97 is 0.1% from 30 and must never snap to 30.
        assert_ne!(snap(29.97).0, Rate::FPS_30);
    }

    #[test]
    fn other_floats_become_simplest_rationals() {
        assert_eq!(
            snap(12.5),
            (Rate::new(25, 2).unwrap(), RateSource::Approximated)
        );
        assert_eq!(
            snap(15.0),
            (Rate::integer(15).unwrap(), RateSource::Approximated)
        );
        assert_eq!(
            snap(1.0 / 3.0),
            (Rate::new(1, 3).unwrap(), RateSource::Approximated)
        );
        assert_eq!(snap(7.0 / 11.0).0, Rate::new(7, 11).unwrap());
        // 29.0 is not a supported rate and is an integer: 29/1.
        assert_eq!(snap(29.0).0, Rate::integer(29).unwrap());
        // 0.1 within 1e-9 is 1/10.
        assert_eq!(snap(0.1).0, Rate::new(1, 10).unwrap());
        // A float that needs a large denominator stays exact within 1e-9.
        let fps = 17.123456789;
        let r = snap(fps).0;
        assert!((r.to_f64() - fps).abs() <= 1e-9);
    }

    #[test]
    fn simplest_rational_is_minimal() {
        // Brute force: no smaller denominator reaches within tolerance.
        for fps in [
            0.7,
            5.4321,
            17.123456789,
            33.3333333333,
            1234.56789,
            0.001234,
        ] {
            let tol = 1e-9;
            let r = simplest_rational_near(fps, tol).unwrap();
            assert!((r.to_f64() - fps).abs() <= tol * 1.000001, "{fps} -> {r}");
            for q in 1..r.den().min(200_000) {
                let p = (fps * q as f64).round();
                assert!(
                    (p / q as f64 - fps).abs() > tol * 0.999999,
                    "{fps}: {p}/{q} beats {r}"
                );
            }
        }
    }

    #[test]
    fn document_rate_prefers_fps_rational() {
        let r = rate_from_document(Some([30000, 1001]), 30.0).unwrap();
        assert_eq!((r.rate, r.source), (Rate::FPS_29_97, RateSource::Exact));
        let r = rate_from_document(Some([0, 1]), 29.97).unwrap();
        assert_eq!((r.rate, r.source), (Rate::FPS_29_97, RateSource::Ntsc));
        let r = rate_from_document(Some([24, 0]), 24.0).unwrap();
        assert_eq!(r.source, RateSource::Integer);
        let r = rate_from_document(None, 25.0).unwrap();
        assert_eq!(r.rate, Rate::FPS_25);
    }

    #[test]
    fn rejects_invalid_floats() {
        for fps in [0.0, -24.0, f64::NAN, f64::INFINITY, 1e12, 1e-12] {
            assert_eq!(
                snap_float_rate(fps),
                Err(TimeError::InvalidFloatRate),
                "{fps}"
            );
        }
    }
}
