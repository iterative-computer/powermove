//! Compile-time parsing of C hexadecimal float literals, so tables ported from
//! C keep their exact source text.

/// Parses `[-]0x<hex>[.<hex>]p<exp>` or a decimal integer (`0`, `0.0`, `1.0`).
///
/// The literal must be exactly representable as a normal `f64` (at most 53
/// significant bits); anything else fails at compile time.
pub(crate) const fn hexf(text: &str) -> f64 {
    let s = text.as_bytes();
    let mut i = 0;
    let negative = !s.is_empty() && s[0] == b'-';
    if negative {
        i = 1;
    }
    // A decimal integer such as `0`, `0.0`, or `1.0`.
    if i < s.len()
        && s[i].is_ascii_digit()
        && !(i + 1 < s.len() && (s[i + 1] == b'x' || s[i + 1] == b'X'))
    {
        let mut n: u64 = 0;
        while i < s.len() && s[i].is_ascii_digit() {
            n = n * 10 + (s[i] - b'0') as u64;
            assert!(n <= 1 << 53, "hexf: decimal integer too large");
            i += 1;
        }
        if i < s.len() {
            assert!(
                s[i] == b'.',
                "hexf: only integers may be written in decimal"
            );
            i += 1;
            while i < s.len() {
                assert!(
                    s[i] == b'0',
                    "hexf: only integers may be written in decimal"
                );
                i += 1;
            }
        }
        let v = n as f64;
        return if negative { -v } else { v };
    }
    assert!(
        i + 1 < s.len() && s[i] == b'0' && (s[i + 1] == b'x' || s[i + 1] == b'X'),
        "hexf: expected 0x"
    );
    i += 2;
    let mut mantissa: u64 = 0;
    let mut bits_after_point: i32 = 0;
    let mut seen_point = false;
    let mut digits = 0;
    while i < s.len() && s[i] != b'p' && s[i] != b'P' {
        let c = s[i];
        if c == b'.' {
            assert!(!seen_point, "hexf: two points");
            seen_point = true;
        } else {
            let d = match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                b'A'..=b'F' => c - b'A' + 10,
                _ => panic!("hexf: bad digit"),
            } as u64;
            assert!(mantissa >> 56 == 0, "hexf: too many digits");
            mantissa = (mantissa << 4) | d;
            digits += 1;
            if seen_point {
                bits_after_point += 4;
            }
        }
        i += 1;
    }
    assert!(digits > 0, "hexf: no digits");
    assert!(i < s.len(), "hexf: missing exponent");
    i += 1;
    let mut exp_negative = false;
    if s[i] == b'-' || s[i] == b'+' {
        exp_negative = s[i] == b'-';
        i += 1;
    }
    let mut exponent: i32 = 0;
    assert!(i < s.len(), "hexf: empty exponent");
    while i < s.len() {
        assert!(s[i].is_ascii_digit(), "hexf: bad exponent digit");
        exponent = exponent * 10 + (s[i] - b'0') as i32;
        i += 1;
    }
    if exp_negative {
        exponent = -exponent;
    }
    if mantissa == 0 {
        return if negative { -0.0 } else { 0.0 };
    }
    // value = mantissa * 2^(exponent - bits_after_point)
    let lz = mantissa.leading_zeros() as i32;
    let top = 63 - lz; // index of the leading one
    let unbiased = top + exponent - bits_after_point;
    assert!(
        unbiased >= -1022 && unbiased <= 1023,
        "hexf: not a normal f64"
    );
    // Align the leading one to bit 52.
    let fraction = if top > 52 {
        let shift = top - 52;
        assert!(
            mantissa & ((1u64 << shift) - 1) == 0,
            "hexf: more than 53 significant bits"
        );
        mantissa >> shift
    } else {
        mantissa << (52 - top)
    };
    let bits = ((negative as u64) << 63)
        | (((unbiased + 1023) as u64) << 52)
        | (fraction & ((1u64 << 52) - 1));
    f64::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::hexf;

    #[test]
    fn parses_c_literals() {
        assert_eq!(hexf("0x1p0"), 1.0);
        assert_eq!(hexf("0x1.0p-1"), 0.5);
        assert_eq!(hexf("0x1.921fb54442d18p0"), core::f64::consts::FRAC_PI_2);
        assert_eq!(hexf("-0x1.8p1"), -3.0);
        assert_eq!(hexf("0x1.fcp-1"), 0.9921875);
        assert_eq!(hexf("0.0").to_bits(), 0);
        assert_eq!(hexf("0").to_bits(), 0);
        assert_eq!(hexf("1.0"), 1.0);
        assert_eq!(hexf("0x1.72c7ba21p-7"), 0.011315313227896695);
        assert_eq!(hexf("0x1.1a62633145c07p-54"), 6.123233995736766e-17);
    }
}
