use core::fmt;

use crate::rational::ceil_div;
use crate::{Rational, TICKS_PER_SECOND, Tick, TimeError};

/// A frame or sample rate: a positive rational of at most one frame per tick.
///
/// The upper bound is what makes `frame_of(frame_start(n)) == n` hold for every
/// `n`: a frame is never shorter than a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rate(Rational);

const fn r(num: i64, den: i64) -> Rate {
    Rate(Rational::from_reduced(num, den))
}

impl Rate {
    pub const FPS_23_976: Rate = r(24_000, 1001);
    pub const FPS_24: Rate = r(24, 1);
    pub const FPS_25: Rate = r(25, 1);
    pub const FPS_29_97: Rate = r(30_000, 1001);
    pub const FPS_30: Rate = r(30, 1);
    pub const FPS_47_952: Rate = r(48_000, 1001);
    pub const FPS_48: Rate = r(48, 1);
    pub const FPS_50: Rate = r(50, 1);
    pub const FPS_59_94: Rate = r(60_000, 1001);
    pub const FPS_60: Rate = r(60, 1);
    pub const FPS_90: Rate = r(90, 1);
    pub const FPS_100: Rate = r(100, 1);
    pub const FPS_119_88: Rate = r(120_000, 1001);
    pub const FPS_120: Rate = r(120, 1);
    pub const FPS_239_76: Rate = r(240_000, 1001);
    pub const FPS_240: Rate = r(240, 1);

    /// A rate `num / den` (reduced). Fails unless `0 < num/den <= TICKS_PER_SECOND`.
    pub fn new(num: i64, den: i64) -> Result<Rate, TimeError> {
        Rate::from_rational(Rational::new(num, den)?)
    }

    /// Validates a rational as a rate.
    pub fn from_rational(value: Rational) -> Result<Rate, TimeError> {
        if value.num() <= 0 || value.num() as i128 > TICKS_PER_SECOND as i128 * value.den() as i128
        {
            return Err(TimeError::InvalidRate);
        }
        Ok(Rate(value))
    }

    /// An integer rate.
    pub fn integer(n: i64) -> Result<Rate, TimeError> {
        Rate::new(n, 1)
    }

    /// The exact rational.
    pub const fn rational(self) -> Rational {
        self.0
    }

    pub const fn num(self) -> i64 {
        self.0.num()
    }

    pub const fn den(self) -> i64 {
        self.0.den()
    }

    /// Nearest `f64`.
    pub fn to_f64(self) -> f64 {
        self.0.to_f64()
    }

    /// The first tick of frame `n`: `ceil(n × den × TICKS / num)`.
    ///
    /// Fails with [`TimeError::Overflow`] when the tick does not fit in `i64`.
    pub fn frame_start(self, n: i64) -> Result<Tick, TimeError> {
        let numerator = (n as i128)
            .checked_mul(self.den() as i128)
            .and_then(|v| v.checked_mul(TICKS_PER_SECOND as i128))
            .ok_or(TimeError::Overflow)?;
        let tick = ceil_div(numerator, self.num() as i128);
        Tick::try_from(tick).map_err(|_| TimeError::Overflow)
    }

    /// The frame containing tick `t`: `(t × num).div_euclid(den × TICKS)`.
    ///
    /// Always fits: a rate is at most one frame per tick.
    pub fn frame_of(self, t: Tick) -> i64 {
        let frame = (t as i128 * self.num() as i128)
            .div_euclid(self.den() as i128 * TICKS_PER_SECOND as i128);
        frame as i64
    }

    /// Ticks per frame when the rate divides the flick exactly.
    pub fn exact_frame_ticks(self) -> Option<Tick> {
        let numerator = self.den() as i128 * TICKS_PER_SECOND as i128;
        (numerator % self.num() as i128 == 0).then(|| (numerator / self.num() as i128) as Tick)
    }

    /// Whether every frame boundary lands on a whole tick (rule 1: new rates must).
    pub fn divides_flick(self) -> bool {
        self.exact_frame_ticks().is_some()
    }

    /// Whether this is one of the sixteen supported frame rates.
    pub fn is_supported(self) -> bool {
        SUPPORTED_RATES.contains(&self)
    }

    /// Whether this is an NTSC rate (`k × 1000/1001`).
    pub const fn is_ntsc(self) -> bool {
        self.den() == 1001
    }
}

impl fmt::Display for Rate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// The supported frame rates (section 5), in ascending order.
pub const SUPPORTED_RATES: [Rate; 16] = [
    Rate::FPS_23_976,
    Rate::FPS_24,
    Rate::FPS_25,
    Rate::FPS_29_97,
    Rate::FPS_30,
    Rate::FPS_47_952,
    Rate::FPS_48,
    Rate::FPS_50,
    Rate::FPS_59_94,
    Rate::FPS_60,
    Rate::FPS_90,
    Rate::FPS_100,
    Rate::FPS_119_88,
    Rate::FPS_120,
    Rate::FPS_239_76,
    Rate::FPS_240,
];

/// Common audio sample rates from 8 kHz to 192 kHz; each divides the flick.
pub const SUPPORTED_SAMPLE_RATES: [i64; 12] = [
    8_000, 11_025, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 88_200, 96_000, 176_400, 192_000,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_rate_divides_the_flick() {
        for rate in SUPPORTED_RATES {
            assert!(rate.divides_flick(), "{rate}");
        }
        for sr in SUPPORTED_SAMPLE_RATES {
            assert!(Rate::integer(sr).unwrap().divides_flick(), "{sr}");
        }
        assert_eq!(Rate::FPS_29_97.exact_frame_ticks(), Some(23_543_520));
        assert_eq!(Rate::FPS_24.exact_frame_ticks(), Some(29_400_000));
        assert!(!Rate::integer(7_000).unwrap().divides_flick() || 705_600_000 % 7_000 == 0);
        assert!(!Rate::new(2997, 100).unwrap().divides_flick());
    }

    #[test]
    fn rejects_invalid_rates() {
        assert_eq!(Rate::new(0, 1), Err(TimeError::InvalidRate));
        assert_eq!(Rate::new(-24, 1), Err(TimeError::InvalidRate));
        assert_eq!(
            Rate::new(TICKS_PER_SECOND + 1, 1),
            Err(TimeError::InvalidRate)
        );
        assert!(Rate::new(TICKS_PER_SECOND, 1).is_ok());
        assert_eq!(Rate::new(24, 0), Err(TimeError::ZeroDenominator));
    }

    #[test]
    fn frame_math_matches_the_formula() {
        let rate = Rate::FPS_29_97;
        assert_eq!(rate.frame_start(0).unwrap(), 0);
        assert_eq!(rate.frame_start(1).unwrap(), 23_543_520);
        assert_eq!(rate.frame_start(-1).unwrap(), -23_543_520);
        assert_eq!(rate.frame_of(23_543_519), 0);
        assert_eq!(rate.frame_of(23_543_520), 1);
        assert_eq!(rate.frame_of(-1), -1);
        // A rate that does not divide the flick rounds frame starts up.
        let odd = Rate::new(7, 3).unwrap(); // 7/3 fps: frame n starts at n × 3/7 s.
        assert_eq!(odd.frame_start(1).unwrap(), 302_400_000);
        let odd = Rate::new(1_000_003, 1).unwrap();
        let start = odd.frame_start(1).unwrap();
        assert_eq!(start, 706); // ceil(705_600_000 / 1_000_003) = 706
        assert_eq!(odd.frame_of(start), 1);
        assert_eq!(odd.frame_of(start - 1), 0);
        assert_eq!(odd.frame_start(-1).unwrap(), -705);
        assert_eq!(odd.frame_of(-705), -1);
        assert_eq!(odd.frame_of(-706), -2);
    }

    #[test]
    fn frame_start_reports_overflow() {
        assert_eq!(Rate::FPS_24.frame_start(i64::MAX), Err(TimeError::Overflow));
        assert_eq!(
            Rate::new(1, i64::MAX).unwrap().frame_start(i64::MAX),
            Err(TimeError::Overflow)
        );
        // frame_of never overflows, even at the extremes.
        assert_eq!(
            Rate::new(TICKS_PER_SECOND, 1).unwrap().frame_of(i64::MIN),
            i64::MIN
        );
        assert_eq!(Rate::new(1, i64::MAX).unwrap().frame_of(i64::MAX), 0);
    }
}
