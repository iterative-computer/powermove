//! Conversions at the media and document boundaries.

use crate::{Rational, TICKS_PER_SECOND, Tick, TimeError, round_half_even_div};

/// Converts a media timestamp to ticks with round-half-even.
///
/// `timebase` is seconds per unit (for example `1/90000`). Exact whenever the
/// timebase divides the flick; the media index keeps the native PTS either way.
pub fn ticks_from_timebase(pts: i64, timebase: Rational) -> Result<Tick, TimeError> {
    if timebase.num() <= 0 {
        return Err(TimeError::InvalidTimebase);
    }
    let numerator = (pts as i128)
        .checked_mul(timebase.num() as i128)
        .and_then(|v| v.checked_mul(TICKS_PER_SECOND as i128))
        .ok_or(TimeError::Overflow)?;
    let tick = round_half_even_div(numerator, timebase.den() as i128);
    Tick::try_from(tick).map_err(|_| TimeError::Overflow)
}

/// Converts ticks to a media timestamp in `timebase` units with round-half-even.
pub fn timebase_from_ticks(ticks: Tick, timebase: Rational) -> Result<i64, TimeError> {
    if timebase.num() <= 0 {
        return Err(TimeError::InvalidTimebase);
    }
    let numerator = (ticks as i128)
        .checked_mul(timebase.den() as i128)
        .ok_or(TimeError::Overflow)?;
    let denominator = (timebase.num() as i128)
        .checked_mul(TICKS_PER_SECOND as i128)
        .ok_or(TimeError::Overflow)?;
    let pts = round_half_even_div(numerator, denominator);
    i64::try_from(pts).map_err(|_| TimeError::Overflow)
}

/// Decomposes a finite `f64` into `mantissa × 2^exponent` exactly.
pub(crate) fn decompose(value: f64) -> (i64, i32) {
    let bits = value.to_bits();
    let sign = if bits >> 63 == 0 { 1i64 } else { -1 };
    let exponent_bits = ((bits >> 52) & 0x7ff) as i32;
    let fraction = (bits & 0x000f_ffff_ffff_ffff) as i64;
    if exponent_bits == 0 {
        (sign * fraction, -1074)
    } else {
        (sign * (fraction | (1 << 52)), exponent_bits - 1075)
    }
}

/// The nearest tick to `seconds` (ties to even), computed exactly from the
/// float's binary value, never through a rounded `seconds × TICKS` product.
pub fn ticks_from_seconds(seconds: f64) -> Result<Tick, TimeError> {
    if !seconds.is_finite() {
        return Err(TimeError::NonFiniteSeconds);
    }
    if seconds == 0.0 {
        return Ok(0);
    }
    let (mantissa, exponent) = decompose(seconds);
    let scaled = mantissa as i128 * TICKS_PER_SECOND as i128; // < 2^83
    let tick = if exponent >= 0 {
        // |seconds| >= 2^52 already overflows i64 ticks (2^52 s × 7e8).
        if exponent > 40 {
            return Err(TimeError::Overflow);
        }
        scaled
            .checked_shl(exponent as u32)
            .ok_or(TimeError::Overflow)?
    } else {
        let shift = (-exponent) as u32;
        if shift >= 120 {
            // |value| < 2^-67 s: far below half a tick.
            0
        } else {
            round_half_even_div(scaled, 1i128 << shift)
        }
    };
    Tick::try_from(tick).map_err(|_| TimeError::Overflow)
}

/// `ticks / TICKS_PER_SECOND` rounded correctly to the nearest `f64`
/// (ties to even), on every target.
pub fn seconds_from_ticks(ticks: Tick) -> f64 {
    const EXACT: i64 = 1 << 53;
    if ticks.unsigned_abs() <= EXACT as u64 {
        // Both operands are exact, so IEEE division rounds once, correctly.
        return ticks as f64 / TICKS_PER_SECOND as f64;
    }
    nearest_f64_of_ratio(ticks as i128, TICKS_PER_SECOND as i128)
}

/// Nearest `f64` to `p / q` (ties to even) for `q > 0` and `2^23 <= |p / q| < 2^40`,
/// where every candidate has a binary exponent of at least -30.
fn nearest_f64_of_ratio(p: i128, q: i128) -> f64 {
    const SCALE: i32 = 30;
    debug_assert!(q > 0);
    let approx = p as f64 / q as f64;
    // |candidate − p/q| × q × 2^SCALE, exact in i128.
    let scaled_error = |candidate: f64| -> i128 {
        let (m, e) = decompose(candidate);
        debug_assert!(e + SCALE >= 0);
        (((m as i128) << (e + SCALE)) * q - (p << SCALE)).abs()
    };
    let mut best = approx;
    let mut best_error = scaled_error(approx);
    for candidate in [approx.next_down(), approx.next_up()] {
        let error = scaled_error(candidate);
        if error < best_error || (error == best_error && decompose(candidate).0 & 1 == 0) {
            best = candidate;
            best_error = error;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timebase_round_trips_when_exact() {
        let tb = Rational::from_reduced(1, 90_000);
        assert_eq!(ticks_from_timebase(90_000, tb).unwrap(), TICKS_PER_SECOND);
        assert_eq!(ticks_from_timebase(1, tb).unwrap(), 7_840);
        assert_eq!(timebase_from_ticks(7_840, tb).unwrap(), 1);
        assert_eq!(ticks_from_timebase(-3, tb).unwrap(), -23_520);
    }

    #[test]
    fn timebase_rounds_half_even() {
        // 1/1_411_200_000 s per unit: half a tick per unit.
        let tb = Rational::from_reduced(1, 1_411_200_000);
        assert_eq!(ticks_from_timebase(1, tb).unwrap(), 0);
        assert_eq!(ticks_from_timebase(3, tb).unwrap(), 2);
        assert_eq!(ticks_from_timebase(5, tb).unwrap(), 2);
        assert_eq!(ticks_from_timebase(-1, tb).unwrap(), 0);
        assert_eq!(ticks_from_timebase(-3, tb).unwrap(), -2);
        // A timebase that does not divide the flick: 1/1_000_003.
        let tb = Rational::from_reduced(1, 1_000_003);
        let t = ticks_from_timebase(1_000_003, tb).unwrap();
        assert_eq!(t, TICKS_PER_SECOND);
        assert_eq!(timebase_from_ticks(t, tb).unwrap(), 1_000_003);
        assert_eq!(ticks_from_timebase(1, tb).unwrap(), 706); // 705.5978…
        assert_eq!(timebase_from_ticks(706, tb).unwrap(), 1);
        assert_eq!(
            ticks_from_timebase(1, Rational::from_reduced(-1, 3)),
            Err(TimeError::InvalidTimebase)
        );
    }

    #[test]
    fn seconds_to_nearest_tick_exactly() {
        assert_eq!(ticks_from_seconds(1.0).unwrap(), TICKS_PER_SECOND);
        assert_eq!(
            ticks_from_seconds(-2.5).unwrap(),
            -2 * TICKS_PER_SECOND - TICKS_PER_SECOND / 2
        );
        assert_eq!(ticks_from_seconds(0.1).unwrap(), 70_560_000);
        // 1e8 s is beyond f64-exact products (7.056e16 > 2^53) but exact here.
        assert_eq!(ticks_from_seconds(1e8).unwrap(), 70_560_000_000_000_000);
        assert_eq!(ticks_from_seconds(-1e8).unwrap(), -70_560_000_000_000_000);
        // Half a tick rounds to even.
        assert_eq!(
            ticks_from_seconds(0.5 / TICKS_PER_SECOND as f64).unwrap(),
            0
        );
        assert_eq!(ticks_from_seconds(f64::MIN_POSITIVE).unwrap(), 0);
        assert_eq!(ticks_from_seconds(5e-324).unwrap(), 0);
        assert_eq!(ticks_from_seconds(1e300), Err(TimeError::Overflow));
        assert_eq!(
            ticks_from_seconds(f64::NAN),
            Err(TimeError::NonFiniteSeconds)
        );
        assert_eq!(ticks_from_seconds(-0.0).unwrap(), 0);
    }

    #[test]
    fn ticks_to_seconds_correctly_rounded() {
        assert_eq!(seconds_from_ticks(TICKS_PER_SECOND), 1.0);
        assert_eq!(seconds_from_ticks(70_560_000), 0.1);
        assert_eq!(seconds_from_ticks(-23_543_520), -1001.0 / 30000.0);
        for t in [
            i64::MAX,
            (1i64 << 53) + 1,
            (1 << 60) + 12_345,
            -(1 << 58) - 7,
            i64::MIN,
            9_007_199_254_740_993,
        ] {
            let s = seconds_from_ticks(t);
            // Independent check: s = m·2^e is correctly rounded iff
            // |m·TICKS − t·2^-e| <= TICKS/2, with m even on a tie.
            let (m, e) = decompose(s);
            assert!(e < 0 && m.unsigned_abs() >= 1 << 52);
            let diff = (m as i128 * TICKS_PER_SECOND as i128 - ((t as i128) << (-e) as u32)).abs();
            let half = TICKS_PER_SECOND as i128 / 2;
            assert!(diff < half || (diff == half && m & 1 == 0), "{t}: {s}");
        }
    }
}
