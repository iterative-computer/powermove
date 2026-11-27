//! The JavaScript string and number semantics the ported loaders depend on
//! (`trim`, `\s`, `parseFloat`, `parseInt`, `Math.min`/`Math.max`,
//! `decodeURIComponent`), so inputs parse as they do in the app.

/// ECMAScript WhiteSpace and LineTerminator (what `trim` and `\s` match):
/// unlike Rust's `char::is_whitespace`, U+FEFF is included and U+0085 is not.
pub fn is_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

pub fn trim(s: &str) -> &str {
    s.trim_matches(is_ws)
}

pub fn trim_start(s: &str) -> &str {
    s.trim_start_matches(is_ws)
}

/// `s.split(/\s+/)`.
pub fn split_ws(s: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut start = 0;
    let mut in_ws = false;
    for (i, c) in s.char_indices() {
        if is_ws(c) {
            if !in_ws {
                out.push(&s[start..i]);
                in_ws = true;
            }
        } else if in_ws {
            start = i;
            in_ws = false;
        }
    }
    out.push(if in_ws { "" } else { &s[start..] });
    out
}

/// The length of the longest prefix of `s` that is a StrDecimalLiteral
/// without sign handling beyond one leading `+` or `-` (0 when none).
fn decimal_prefix(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    if s[i..].starts_with("Infinity") {
        return i + 8;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut digits = i > int_start;
    if i < b.len() && b[i] == b'.' {
        let frac_start = i + 1;
        let mut j = frac_start;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if digits || j > frac_start {
            digits = true;
            i = j;
        }
    }
    if !digits {
        return 0;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    i
}

fn decimal_value(s: &str) -> f64 {
    let unsigned = s.trim_start_matches(['+', '-']);
    let negative = s.starts_with('-');
    let v = if unsigned == "Infinity" {
        f64::INFINITY
    } else {
        // Rust rejects a trailing `.` before an exponent ("5.e3"); JS does not.
        let fixed;
        let text = if unsigned.contains(".e") || unsigned.contains(".E") || unsigned.ends_with('.')
        {
            fixed = unsigned.replacen('.', ".0", 1);
            fixed.as_str()
        } else {
            unsigned
        };
        text.parse::<f64>().unwrap_or(f64::NAN)
    };
    if negative { -v } else { v }
}

/// `parseFloat(s)`.
pub fn parse_float(s: &str) -> f64 {
    let s = trim_start(s);
    match decimal_prefix(s) {
        0 => f64::NAN,
        n => decimal_value(&s[..n]),
    }
}

/// `parseInt(s, 10)`.
pub fn parse_int(s: &str) -> f64 {
    let s = trim_start(s);
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let n = digits.bytes().take_while(u8::is_ascii_digit).count();
    if n == 0 {
        return f64::NAN;
    }
    let v: f64 = digits[..n].parse().unwrap_or(f64::NAN);
    if negative { -v } else { v }
}

/// `Math.min(a, b)` (NaN wins).
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// `Math.max(a, b)` (NaN wins).
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// Uint8ClampedArray assignment (ToUint8Clamp: round half to even).
pub fn u8_clamp(v: f64) -> u8 {
    if v.is_nan() || v <= 0.0 {
        0
    } else if v >= 255.0 {
        255
    } else {
        v.round_ties_even() as u8
    }
}

/// `decodeURIComponent`.
pub fn decode_uri_component(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_parse_as_javascript_parses_them() {
        assert_eq!(parse_float("1.5abc"), 1.5);
        assert_eq!(parse_float("  -2e2x"), -200.0);
        assert_eq!(parse_float("5.e1"), 50.0);
        assert!(parse_float("x1").is_nan());
        assert_eq!(parse_float("1e"), 1.0);
        assert_eq!(parse_int("0x1"), 0.0);
        assert_eq!(parse_int("-12.7"), -12.0);
        assert!(parse_int("").is_nan());
    }

    #[test]
    fn whitespace_and_helpers_follow_javascript() {
        assert_eq!(trim("\u{FEFF} a \u{85}"), "a \u{85}");
        assert_eq!(split_ws("a  b\tc "), vec!["a", "b", "c", ""]);
        assert_eq!(split_ws(""), vec![""]);
        assert_eq!(u8_clamp(127.5), 128);
        assert_eq!(u8_clamp(126.5), 126);
        assert_eq!(decode_uri_component("a%20b").as_deref(), Some("a b"));
        assert_eq!(decode_uri_component("a%2"), None);
    }
}
