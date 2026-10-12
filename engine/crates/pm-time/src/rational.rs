use core::cmp::Ordering;
use core::fmt;

use crate::TimeError;

/// An exact rational number `num / den` in lowest terms with `den > 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rational {
    num: i64,
    den: i64,
}

const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

impl Rational {
    /// Zero.
    pub const ZERO: Rational = Rational { num: 0, den: 1 };

    /// `num / den` reduced to lowest terms with a positive denominator.
    pub fn new(num: i64, den: i64) -> Result<Rational, TimeError> {
        if den == 0 {
            return Err(TimeError::ZeroDenominator);
        }
        let g = gcd(num.unsigned_abs(), den.unsigned_abs()) as i128;
        let (mut n, mut d) = (num as i128 / g, den as i128 / g);
        if d < 0 {
            n = -n;
            d = -d;
        }
        // Only i64::MIN / -1 style inputs overflow after negation.
        let num = i64::try_from(n).map_err(|_| TimeError::Overflow)?;
        let den = i64::try_from(d).map_err(|_| TimeError::Overflow)?;
        Ok(Rational { num, den })
    }

    /// A rational from parts already in lowest terms with `den > 0`.
    ///
    /// Used for constants; `debug_assert`s the invariant.
    pub const fn from_reduced(num: i64, den: i64) -> Rational {
        debug_assert!(den > 0);
        debug_assert!(gcd(num.unsigned_abs(), den.unsigned_abs()) == 1);
        Rational { num, den }
    }

    /// An integer.
    pub const fn integer(n: i64) -> Rational {
        Rational { num: n, den: 1 }
    }

    /// Numerator (carries the sign).
    pub const fn num(self) -> i64 {
        self.num
    }

    /// Denominator (always positive).
    pub const fn den(self) -> i64 {
        self.den
    }

    /// Nearest `f64` (one rounding when both parts are exactly representable).
    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Whether the value is an integer.
    pub const fn is_integer(self) -> bool {
        self.den == 1
    }

    /// `[num, den]`, the document's `fpsRational` shape.
    pub const fn to_pair(self) -> [i64; 2] {
        [self.num, self.den]
    }
}

impl PartialOrd for Rational {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rational {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.num as i128 * other.den as i128).cmp(&(other.num as i128 * self.den as i128))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

/// `p / q` rounded to the nearest integer, ties to even, for `q > 0`.
///
/// # Panics
///
/// Panics in debug builds if `q <= 0`.
pub fn round_half_even_div(p: i128, q: i128) -> i128 {
    debug_assert!(q > 0);
    let floor = p.div_euclid(q);
    let rem = p.rem_euclid(q);
    // Compare 2·rem with q without overflowing: rem < q, so q - rem > 0.
    match rem.cmp(&(q - rem)) {
        Ordering::Less => floor,
        Ordering::Greater => floor + 1,
        Ordering::Equal => floor + (floor & 1),
    }
}

/// `ceil(p / q)` for `q > 0`.
pub(crate) fn ceil_div(p: i128, q: i128) -> i128 {
    debug_assert!(q > 0);
    -((-p).div_euclid(q))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduces_and_normalizes_sign() {
        assert_eq!(
            Rational::new(60000, 2002).unwrap(),
            Rational::from_reduced(30000, 1001)
        );
        assert_eq!(Rational::new(3, -6).unwrap().to_pair(), [-1, 2]);
        assert_eq!(Rational::new(0, -5).unwrap(), Rational::ZERO);
        assert_eq!(Rational::new(1, 0), Err(TimeError::ZeroDenominator));
        assert_eq!(Rational::new(i64::MIN, -1), Err(TimeError::Overflow));
        assert_eq!(
            Rational::new(i64::MIN, 2).unwrap().to_pair(),
            [i64::MIN / 2, 1]
        );
    }

    #[test]
    fn orders_exactly() {
        let a = Rational::from_reduced(30000, 1001);
        let b = Rational::from_reduced(2997, 100);
        assert!(a > b);
        assert!(Rational::integer(-1) < Rational::ZERO);
    }

    #[test]
    fn round_half_even_cases() {
        assert_eq!(round_half_even_div(5, 2), 2);
        assert_eq!(round_half_even_div(7, 2), 4);
        assert_eq!(round_half_even_div(-5, 2), -2);
        assert_eq!(round_half_even_div(-7, 2), -4);
        assert_eq!(round_half_even_div(10, 3), 3);
        assert_eq!(round_half_even_div(11, 3), 4);
        assert_eq!(round_half_even_div(-10, 3), -3);
        assert_eq!(round_half_even_div(-11, 3), -4);
        assert_eq!(round_half_even_div(0, 7), 0);
    }

    #[test]
    fn ceil_div_cases() {
        assert_eq!(ceil_div(7, 2), 4);
        assert_eq!(ceil_div(-7, 2), -3);
        assert_eq!(ceil_div(6, 2), 3);
        assert_eq!(ceil_div(-6, 2), -3);
    }
}
