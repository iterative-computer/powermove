//! JavaScript value semantics that evaluation depends on (plan section 7).
//!
//! The legacy evaluator runs on raw `JSON.parse` output, so a key whose value
//! is a string, a boolean, or `null` flows through JavaScript's operators:
//! `"5" + 2.5` is `"52.5"`, `"5" - 1` is `4`, and `null * 2` is `0`. Projects
//! saved with such values evaluate the way V8 evaluated them, so the engine
//! reproduces the operators here instead of rejecting the input.
//!
//! * [`JsValue`]: a JavaScript value as evaluation sees it (`undefined`,
//!   primitives, and JSON arrays and objects).
//! * Conversions: [`JsValue::to_number`] (`ToNumber`), [`JsValue::to_js_string`]
//!   (`ToString`), [`JsValue::truthy`] (`ToBoolean`), [`JsValue::type_of`].
//! * Operators: [`add`] (`+` with string concatenation), [`sub`], [`mul`],
//!   [`div`], [`rem`] (`%`, the sign of the dividend), [`loose_eq`] (`==`),
//!   [`strict_eq`] (`===`), [`less_than`] and friends (relational comparison).
//! * Library functions: [`math_round`] (`Math.round`, halves toward +∞),
//!   [`parse_int`], [`number_to_string_radix`] (`Number.prototype.toString(radix)`,
//!   V8's algorithm), [`pad_start`].
//!
//! Every function is tested against V8 (`tests/vectors/js-sem-vectors.json`,
//! written by `tests/vectors/generate-js-sem-vectors.mjs` under Node).
//!
//! Arrays and objects have no identity here: two [`JsValue::Object`]s are never
//! `==` or `===` (two distinct JavaScript objects are not either; the engine
//! cannot tell whether two values came from the same object, so it answers as
//! for distinct ones).

use pm_model::json::{number_to_js_string, string_to_number};
use pm_model::{JsString, Value};

/// A JavaScript value.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum JsValue {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(JsString),
    /// A JSON array or plain object (always [`Value::Array`] or
    /// [`Value::Object`]).
    Object(Box<Value>),
}

impl From<f64> for JsValue {
    fn from(n: f64) -> JsValue {
        JsValue::Number(n)
    }
}

impl From<bool> for JsValue {
    fn from(b: bool) -> JsValue {
        JsValue::Bool(b)
    }
}

impl From<&str> for JsValue {
    fn from(s: &str) -> JsValue {
        JsValue::String(JsString::from(s))
    }
}

impl From<JsString> for JsValue {
    fn from(s: JsString) -> JsValue {
        JsValue::String(s)
    }
}

impl From<Value> for JsValue {
    fn from(v: Value) -> JsValue {
        match v {
            Value::Null => JsValue::Null,
            Value::Bool(b) => JsValue::Bool(b),
            Value::Number(n) => JsValue::Number(n),
            Value::String(s) => JsValue::String(s),
            other => JsValue::Object(Box::new(other)),
        }
    }
}

impl From<&Value> for JsValue {
    fn from(v: &Value) -> JsValue {
        match v {
            Value::Null => JsValue::Null,
            Value::Bool(b) => JsValue::Bool(*b),
            Value::Number(n) => JsValue::Number(*n),
            Value::String(s) => JsValue::String(s.clone()),
            other => JsValue::Object(Box::new(other.clone())),
        }
    }
}

impl JsValue {
    /// The value as JSON, the way `JSON.stringify` would write it inside an
    /// object: `undefined` has no JSON form (`None`); non-finite numbers
    /// become `null`.
    pub fn to_json(&self) -> Option<Value> {
        Some(match self {
            JsValue::Undefined => return None,
            JsValue::Null => Value::Null,
            JsValue::Bool(b) => Value::Bool(*b),
            JsValue::Number(n) if n.is_finite() => Value::Number(*n),
            JsValue::Number(_) => Value::Null,
            JsValue::String(s) => Value::String(s.clone()),
            JsValue::Object(v) => (**v).clone(),
        })
    }

    /// The number, when this is a number.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            JsValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// `typeof value`.
    pub fn type_of(&self) -> &'static str {
        match self {
            JsValue::Undefined => "undefined",
            JsValue::Null | JsValue::Object(_) => "object",
            JsValue::Bool(_) => "boolean",
            JsValue::Number(_) => "number",
            JsValue::String(_) => "string",
        }
    }

    /// `typeof value === 'number' && Number.isFinite(value)`.
    pub fn is_finite_number(&self) -> bool {
        matches!(self, JsValue::Number(n) if n.is_finite())
    }

    /// `!!value` (`ToBoolean`).
    pub fn truthy(&self) -> bool {
        match self {
            JsValue::Undefined | JsValue::Null => false,
            JsValue::Bool(b) => *b,
            JsValue::Number(n) => *n != 0.0 && !n.is_nan(),
            JsValue::String(s) => !s.as_bytes().is_empty(),
            JsValue::Object(_) => true,
        }
    }

    /// `ToPrimitive` with the default or number hint. Arrays and plain
    /// objects have no own `valueOf`, so they become their `toString`:
    /// `Array.prototype.join` or `"[object Object]"`.
    pub fn to_primitive(&self) -> JsValue {
        match self {
            JsValue::Object(v) => JsValue::String(v.to_js_string()),
            other => other.clone(),
        }
    }

    /// `ToNumber` (`Number(value)`, unary `+`).
    pub fn to_number(&self) -> f64 {
        match self {
            JsValue::Undefined => f64::NAN,
            JsValue::Null => 0.0,
            JsValue::Bool(b) => f64::from(u8::from(*b)),
            JsValue::Number(n) => *n,
            JsValue::String(s) => js_string_to_number(s),
            JsValue::Object(v) => js_string_to_number(&v.to_js_string()),
        }
    }

    /// `ToString` (`String(value)`, template literals).
    pub fn to_js_string(&self) -> JsString {
        match self {
            JsValue::Undefined => JsString::from("undefined"),
            JsValue::Null => JsString::from("null"),
            JsValue::Bool(b) => JsString::from(if *b { "true" } else { "false" }),
            JsValue::Number(n) => JsString::from(number_to_js_string(*n)),
            JsValue::String(s) => s.clone(),
            JsValue::Object(v) => v.to_js_string(),
        }
    }

    /// `value ?? fallback`.
    pub fn or_nullish(self, fallback: JsValue) -> JsValue {
        match self {
            JsValue::Undefined | JsValue::Null => fallback,
            other => other,
        }
    }
}

/// `StringToNumber` on a JavaScript string. A string with a lone surrogate is
/// never numeric (surrogates are not whitespace).
pub fn js_string_to_number(s: &JsString) -> f64 {
    match s.as_str() {
        Some(text) => string_to_number(text),
        None => f64::NAN,
    }
}

/// `ToNumber` of a JSON value (`Number(value)`).
pub fn value_to_number(v: &Value) -> f64 {
    match v {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => *n,
        Value::String(s) => js_string_to_number(s),
        other => js_string_to_number(&other.to_js_string()),
    }
}

fn concat(a: &JsString, b: &JsString) -> JsString {
    match (a.as_str(), b.as_str()) {
        (Some(x), Some(y)) => {
            let mut s = String::with_capacity(x.len() + y.len());
            s.push_str(x);
            s.push_str(y);
            JsString::from(s)
        }
        // A lone surrogate at the seam may pair up with its neighbour, as it
        // does in JavaScript.
        _ => {
            let units: Vec<u16> = a.utf16_units().chain(b.utf16_units()).collect();
            JsString::from_utf16_units(&units)
        }
    }
}

/// `a + b`: string concatenation when either primitive is a string, numeric
/// addition otherwise.
pub fn add(a: &JsValue, b: &JsValue) -> JsValue {
    if let (JsValue::Number(x), JsValue::Number(y)) = (a, b) {
        return JsValue::Number(x + y);
    }
    let pa = a.to_primitive();
    let pb = b.to_primitive();
    if matches!(pa, JsValue::String(_)) || matches!(pb, JsValue::String(_)) {
        return JsValue::String(concat(&pa.to_js_string(), &pb.to_js_string()));
    }
    JsValue::Number(pa.to_number() + pb.to_number())
}

/// `a - b`.
pub fn sub(a: &JsValue, b: &JsValue) -> f64 {
    a.to_number() - b.to_number()
}

/// `a * b`.
pub fn mul(a: &JsValue, b: &JsValue) -> f64 {
    a.to_number() * b.to_number()
}

/// `a / b`.
pub fn div(a: &JsValue, b: &JsValue) -> f64 {
    a.to_number() / b.to_number()
}

/// `a % b` on numbers: the remainder of truncating division, with the sign
/// of the dividend (`-7 % 3` is `-1`, `-6 % 3` is `-0`). Rust's `%` on `f64`
/// is C's `fmod`, which is exactly ECMAScript's `Number::remainder`.
pub fn rem(a: f64, b: f64) -> f64 {
    a % b
}

/// `a === b`.
pub fn strict_eq(a: &JsValue, b: &JsValue) -> bool {
    match (a, b) {
        (JsValue::Undefined, JsValue::Undefined) | (JsValue::Null, JsValue::Null) => true,
        (JsValue::Bool(x), JsValue::Bool(y)) => x == y,
        (JsValue::Number(x), JsValue::Number(y)) => x == y,
        (JsValue::String(x), JsValue::String(y)) => x == y,
        _ => false,
    }
}

/// `a == b` (IsLooselyEqual, ECMA-262 §7.2.14).
pub fn loose_eq(a: &JsValue, b: &JsValue) -> bool {
    use JsValue::*;
    match (a, b) {
        (Undefined | Null, Undefined | Null) => true,
        (Undefined | Null, _) | (_, Undefined | Null) => false,
        (Object(_), Object(_)) => false,
        (Number(_), String(_)) | (String(_), Number(_)) => a.to_number() == b.to_number(),
        (Bool(x), _) => loose_eq(&Number(f64::from(u8::from(*x))), b),
        (_, Bool(y)) => loose_eq(a, &Number(f64::from(u8::from(*y)))),
        (Object(_), _) => loose_eq(&a.to_primitive(), b),
        (_, Object(_)) => loose_eq(a, &b.to_primitive()),
        _ => strict_eq(a, b),
    }
}

/// The result of IsLessThan: `None` stands for `undefined` (a NaN operand).
fn is_less_than(a: &JsValue, b: &JsValue) -> Option<bool> {
    let pa = a.to_primitive();
    let pb = b.to_primitive();
    if let (JsValue::String(x), JsValue::String(y)) = (&pa, &pb) {
        return Some(x.cmp_utf16(y).is_lt());
    }
    let x = pa.to_number();
    let y = pb.to_number();
    if x.is_nan() || y.is_nan() {
        return None;
    }
    Some(x < y)
}

/// `a < b`.
pub fn less_than(a: &JsValue, b: &JsValue) -> bool {
    is_less_than(a, b) == Some(true)
}

/// `a > b`.
pub fn greater_than(a: &JsValue, b: &JsValue) -> bool {
    is_less_than(b, a) == Some(true)
}

/// `a <= b`.
pub fn less_equal(a: &JsValue, b: &JsValue) -> bool {
    is_less_than(b, a) == Some(false)
}

/// `a >= b`.
pub fn greater_equal(a: &JsValue, b: &JsValue) -> bool {
    is_less_than(a, b) == Some(false)
}

/// `Math.round(x)`: the nearest integer, halves toward +∞; `-0` for inputs in
/// [-0.5, -0]. (`Math.floor(x + 0.5)` is wrong for 0.49999999999999994 and
/// for odd integers above 2^52.)
pub fn math_round(x: f64) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    if x > 0.0 && x < 0.5 {
        return 0.0;
    }
    if x < 0.0 && x >= -0.5 {
        return -0.0;
    }
    let floor = x.floor();
    // Exact: x and floor(x) are within one unit of each other.
    if x - floor >= 0.5 { floor + 1.0 } else { floor }
}

/// `Number.isFinite(x)` on a value (no coercion).
pub fn number_is_finite(v: &JsValue) -> bool {
    v.is_finite_number()
}

/// `parseInt(string, radix)` for radix 2 to 36 (other radices answer NaN;
/// radix 0 or `undefined` is 10 with `0x` detection).
pub fn parse_int(input: &JsString, radix: i32) -> f64 {
    let text = input.to_string_lossy();
    let mut s = pm_model::json::js_trim(&text);
    let mut sign = 1.0;
    if let Some(rest) = s.strip_prefix('-') {
        sign = -1.0;
        s = rest;
    } else if let Some(rest) = s.strip_prefix('+') {
        s = rest;
    }
    let mut radix = radix;
    let mut strip_prefix = true;
    if radix != 0 {
        if !(2..=36).contains(&radix) {
            return f64::NAN;
        }
        if radix != 16 {
            strip_prefix = false;
        }
    } else {
        radix = 10;
    }
    if strip_prefix && (s.starts_with("0x") || s.starts_with("0X")) {
        s = &s[2..];
        radix = 16;
    }
    let digits: Vec<u32> = s
        .chars()
        .map_while(|c| c.to_digit(radix as u32))
        .collect();
    if digits.is_empty() {
        return f64::NAN;
    }
    if radix == 10 {
        // Decimal digits are rounded as a decimal literal, like V8 does.
        let literal: String = s.chars().take(digits.len()).collect();
        return sign * literal.parse::<f64>().unwrap_or(f64::NAN);
    }
    let mut value = 0.0f64;
    let mut exact = true;
    for &d in &digits {
        let next = value * radix as f64 + d as f64;
        if next >= 9_007_199_254_740_992.0 {
            exact = false;
        }
        value = next;
    }
    if !exact && radix.count_ones() == 1 {
        // Power-of-two radices round correctly from the binary digits.
        value = power_of_two_radix(&digits, radix as u32);
    }
    sign * value
}

fn power_of_two_radix(digits: &[u32], radix: u32) -> f64 {
    let bits = radix.trailing_zeros();
    let mut mantissa: u128 = 0;
    let mut extra_bits: i32 = 0;
    let mut sticky = false;
    for &d in digits {
        if extra_bits > 0 || mantissa >> (127 - bits) != 0 {
            extra_bits += bits as i32;
            sticky |= d != 0;
        } else {
            mantissa = (mantissa << bits) | d as u128;
        }
    }
    let base = (mantissa | u128::from(sticky)) as f64;
    if extra_bits > 1100 {
        return f64::INFINITY;
    }
    base * 2f64.powi(extra_bits)
}

const RADIX_DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn next_double(x: f64) -> f64 {
    // x is finite and non-negative here.
    f64::from_bits(x.to_bits() + 1)
}

/// The binary exponent of `x` as V8's `Double::Exponent` defines it: `x` is
/// `significand × 2^exponent` with a 53-bit integer significand.
fn double_exponent(x: f64) -> i32 {
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    if biased == 0 { -1074 } else { biased - 1075 }
}

/// `Number.prototype.toString(radix)` for radix 2 to 36, ported from V8's
/// `DoubleToRadixCString` (`src/numbers/conversions.cc`). Radix 10 is
/// [`number_to_js_string`].
///
/// # Panics
///
/// Panics if `radix` is outside 2 to 36.
pub fn number_to_string_radix(value: f64, radix: u32) -> String {
    assert!((2..=36).contains(&radix), "radix {radix} out of range");
    if radix == 10 || value.is_nan() || value.is_infinite() || value == 0.0 {
        return number_to_js_string(value);
    }
    let radix_f = radix as f64;
    let negative = value < 0.0;
    let value = value.abs();
    let mut integer = value.floor();
    let mut fraction = value - integer;
    let mut delta = 0.5 * (next_double(value) - value);
    delta = delta.max(next_double(0.0));
    let mut frac_digits: Vec<u8> = Vec::new();
    if fraction >= delta {
        loop {
            fraction *= radix_f;
            delta *= radix_f;
            let digit = fraction as usize;
            frac_digits.push(RADIX_DIGITS[digit]);
            fraction -= digit as f64;
            if (fraction > 0.5 || (fraction == 0.5 && (digit & 1) == 1)) && fraction + delta > 1.0
            {
                // Round up, propagating the carry through written digits.
                loop {
                    match frac_digits.pop() {
                        None => {
                            integer += 1.0;
                            break;
                        }
                        Some(c) => {
                            let d = if c > b'9' {
                                (c - b'a' + 10) as u32
                            } else {
                                (c - b'0') as u32
                            };
                            if d + 1 < radix {
                                frac_digits.push(RADIX_DIGITS[(d + 1) as usize]);
                                break;
                            }
                        }
                    }
                }
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }
    let mut int_digits: Vec<u8> = Vec::new();
    while double_exponent(integer / radix_f) > 0 {
        integer /= radix_f;
        int_digits.push(b'0');
    }
    loop {
        let remainder = integer % radix_f;
        int_digits.push(RADIX_DIGITS[remainder as usize]);
        integer = (integer - remainder) / radix_f;
        if integer <= 0.0 {
            break;
        }
    }
    let mut out = String::with_capacity(int_digits.len() + frac_digits.len() + 2);
    if negative {
        out.push('-');
    }
    out.extend(int_digits.iter().rev().map(|&b| b as char));
    if !frac_digits.is_empty() {
        out.push('.');
        out.extend(frac_digits.iter().map(|&b| b as char));
    }
    out
}

/// `string.padStart(length, fill)` with a non-empty fill, on code units.
pub fn pad_start(s: &str, length: usize, fill: char) -> String {
    let len = s.encode_utf16().count();
    if len >= length {
        return s.to_owned();
    }
    let mut out: String = std::iter::repeat_n(fill, length - len).collect();
    out.push_str(s);
    out
}

/// A thrown JavaScript exception, where the legacy code would throw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsThrow {
    /// The constructor name (`TypeError`, ...).
    pub kind: &'static str,
    /// A description of where it threw (not V8's message text).
    pub message: String,
}

impl JsThrow {
    pub fn type_error(message: impl Into<String>) -> JsThrow {
        JsThrow {
            kind: "TypeError",
            message: message.into(),
        }
    }
}

impl core::fmt::Display for JsThrow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for JsThrow {}

/// The result of an operation that throws in JavaScript.
pub type JsResult<T> = Result<T, JsThrow>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_halves_toward_positive_infinity() {
        assert_eq!(math_round(0.5), 1.0);
        assert_eq!(math_round(-0.5).to_bits(), (-0.0f64).to_bits());
        assert_eq!(math_round(-1.5), -1.0);
        assert_eq!(math_round(2.5), 3.0);
        assert_eq!(math_round(0.49999999999999994), 0.0);
        assert_eq!(math_round(4503599627370497.0), 4503599627370497.0);
        assert!(math_round(f64::NAN).is_nan());
    }

    #[test]
    fn plus_concatenates_strings() {
        let five = JsValue::from("5");
        assert_eq!(
            add(&five, &JsValue::Number(2.5)),
            JsValue::from("52.5")
        );
        assert_eq!(sub(&five, &JsValue::Number(1.0)), 4.0);
        assert_eq!(add(&JsValue::Null, &JsValue::Number(2.0)), JsValue::Number(2.0));
        assert_eq!(add(&JsValue::Bool(true), &JsValue::Bool(true)), JsValue::Number(2.0));
    }

    #[test]
    fn radix_strings() {
        assert_eq!(number_to_string_radix(255.0, 16), "ff");
        assert_eq!(number_to_string_radix(-255.0, 16), "-ff");
        assert_eq!(number_to_string_radix(0.5, 2), "0.1");
        assert_eq!(number_to_string_radix(-0.0, 16), "0");
    }
}
