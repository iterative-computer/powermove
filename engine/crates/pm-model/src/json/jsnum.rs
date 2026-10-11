//! ECMAScript number conversions.

/// `Number::toString(x)` (ECMA-262 §6.1.6.1.20) for radix 10.
///
/// ECMAScript requires the shortest digit string that round-trips and, when
/// several strings of that length do, the one closest to the exact value
/// (ties to even). Rust's `{:e}` formatting is shortest but not always closest
/// (`656607186168349.2` comes out as `…349.3`), so this uses `ryu-js`, a Ryū
/// port with ECMAScript layout: plain notation for decimal exponents from -7
/// to 20, exponential (`1e+21`, `1.5e-7`) otherwise. A differential run of two
/// million doubles against V8 and JavaScriptCore found no difference.
pub fn number_to_js_string(x: f64) -> String {
    let mut out = String::new();
    push_js_number(x, &mut out);
    out
}

pub(crate) fn push_js_number(x: f64, out: &mut String) {
    out.push_str(ryu_js::Buffer::new().format(x));
}

/// `StringToNumber` (ECMA-262 §7.1.4.1.1): what `Number(s)` returns.
///
/// Surrounding whitespace and line terminators are trimmed; the empty string is
/// 0; `0x`, `0o`, and `0b` integers have no sign; `Infinity` is the only
/// non-numeric word; anything else is NaN.
pub fn string_to_number(s: &str) -> f64 {
    let trimmed = s.trim_matches(is_js_whitespace);
    if trimmed.is_empty() {
        return 0.0;
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() > 2 && bytes[0] == b'0' {
        let radix = match bytes[1] {
            b'x' | b'X' => Some(16),
            b'o' | b'O' => Some(8),
            b'b' | b'B' => Some(2),
            _ => None,
        };
        if let Some(radix) = radix {
            return parse_radix_integer(&trimmed[2..], radix);
        }
    }
    let (sign, body) = match bytes[0] {
        b'+' => (1.0, &trimmed[1..]),
        b'-' => (-1.0, &trimmed[1..]),
        _ => (1.0, trimmed),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    if !is_str_unsigned_decimal_literal(body) {
        return f64::NAN;
    }
    // Rust's parser is correctly rounded and accepts this grammar subset.
    let value: f64 = body.parse().unwrap_or(f64::NAN);
    sign * value
}

/// StrUnsignedDecimalLiteral without `Infinity`: digits with an optional
/// fraction (either side may be empty, not both) and an optional exponent.
fn is_str_unsigned_decimal_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - start;
    }
    if int_digits + frac_digits == 0 {
        return false;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

/// A non-decimal integer literal, rounded to the nearest `f64` like
/// ECMAScript's MV computation (exact, then rounded once).
fn parse_radix_integer(digits: &str, radix: u32) -> f64 {
    if digits.is_empty() {
        return f64::NAN;
    }
    // Accumulate exactly while it fits in u128; beyond that, scale by the
    // radix with the remaining digit count (sticky bits make the final
    // rounding correct for radix 2, 8, and 16).
    let mut value: u128 = 0;
    let mut overflow_digits = 0i32;
    let mut sticky = false;
    for c in digits.chars() {
        let Some(d) = c.to_digit(radix) else {
            return f64::NAN;
        };
        if overflow_digits > 0 || value > (u128::MAX - d as u128) / radix as u128 {
            overflow_digits += 1;
            sticky |= d != 0;
        } else {
            value = value * radix as u128 + d as u128;
        }
    }
    if overflow_digits == 0 {
        return u128_to_f64(value);
    }
    let bits_per_digit = radix.trailing_zeros() as i32;
    let shift = overflow_digits * bits_per_digit;
    let base = u128_to_f64(value | sticky as u128);
    if shift > 1100 {
        return f64::INFINITY;
    }
    base * 2f64.powi(shift)
}

fn u128_to_f64(v: u128) -> f64 {
    // `as` rounds to nearest, ties to even.
    v as f64
}

/// `String.prototype.trim()`.
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// WhiteSpace and LineTerminator code points (ECMA-262 §12.2, §12.3).
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{FEFF}'
            | '\u{000A}'
            | '\u{000D}'
            | '\u{2028}'
            | '\u{2029}'
    ) || is_space_separator(c)
}

/// Unicode category Zs.
fn is_space_separator(c: char) -> bool {
    matches!(
        c,
        '\u{0020}' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

/// The canonical numeric string test for array indices: a property key is an
/// array index when it is `ToString(ToUint32(key))` and below 2^32 − 1.
pub(crate) fn array_index(key: &str) -> Option<u32> {
    let b = key.as_bytes();
    if b.is_empty() || b.len() > 10 || (b.len() > 1 && b[0] == b'0') {
        return None;
    }
    let mut value: u64 = 0;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        value = value * 10 + (c - b'0') as u64;
    }
    (value < u32::MAX as u64).then_some(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_ecmascript() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.5, "-1.5"),
            (0.1, "0.1"),
            (100.0, "100"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1.5e21, "1.5e+21"),
            (123456789012345680000.0, "123456789012345680000"),
            (0.000001, "0.000001"),
            (0.0000001, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (5e-324, "5e-324"),
            (f64::MAX, "1.7976931348623157e+308"),
            (2f64.powi(53), "9007199254740992"),
            (0.1 + 0.2, "0.30000000000000004"),
            (9.766666666666667, "9.766666666666667"),
            (f64::from_bits(0x4302a97230b550ea), "656607186168349.2"),
            (f64::from_bits(0x42d5df704592fa48), "96197621992425.12"),
            (f64::from_bits(0xc302102efb584eca), "-635542944025049.2"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
        ];
        for &(x, expected) in cases {
            assert_eq!(number_to_js_string(x), expected, "{x:e}");
        }
    }

    #[test]
    fn string_to_number_follows_the_grammar() {
        let cases: &[(&str, f64)] = &[
            ("", 0.0),
            ("  \n\t ", 0.0),
            ("12", 12.0),
            (" 12 ", 12.0),
            ("-3.5", -3.5),
            ("+.5", 0.5),
            ("5.", 5.0),
            ("1e3", 1000.0),
            ("1E-2", 0.01),
            ("0x1F", 31.0),
            ("0b101", 5.0),
            ("0o17", 15.0),
            ("Infinity", f64::INFINITY),
            ("-Infinity", f64::NEG_INFINITY),
            ("\u{a0}7\u{2028}", 7.0),
            ("1e400", f64::INFINITY),
        ];
        for &(s, expected) in cases {
            assert_eq!(string_to_number(s), expected, "{s:?}");
        }
        for s in [
            "abc", "1x", ".", "e5", "1e", "-0x1", "0x", "inf", "nan", "infinity", "1_000", "++1",
            "1 2",
        ] {
            assert!(string_to_number(s).is_nan(), "{s:?}");
        }
        assert!(string_to_number("-0").is_sign_negative());
    }

    #[test]
    fn array_indices() {
        assert_eq!(array_index("0"), Some(0));
        assert_eq!(array_index("42"), Some(42));
        assert_eq!(array_index("4294967294"), Some(4_294_967_294));
        assert_eq!(array_index("4294967295"), None);
        for key in ["", "01", "-1", "1.0", "1e3", " 1", "length", "99999999999"] {
            assert_eq!(array_index(key), None, "{key}");
        }
    }
}
